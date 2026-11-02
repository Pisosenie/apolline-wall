use std::ffi::c_void;
use std::num::NonZeroU32;
use std::ptr::NonNull;

use glow::HasContext;
use glutin::api::egl::config::Config;
use glutin::api::egl::context::PossiblyCurrentContext;
use glutin::api::egl::display::Display;
use glutin::api::egl::surface::Surface;
use glutin::config::{Api, ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentGlContext, Version};
use glutin::display::GlDisplay;
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

use crate::picture::Pixels;

pub use glow::Texture;

const VERTEX: &str = "#version 300 es
out vec2 uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    uv = vec2(p.x, 1.0 - p.y);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}";

const FRAGMENT: &str = "#version 300 es
precision highp float;
uniform sampler2D before;
uniform sampler2D after;
uniform float progress;
in vec2 uv;
out vec4 color;
void main() {
    color = vec4(mix(texture(before, uv).rgb, texture(after, uv).rgb, progress), 1.0);
}";

pub struct Gl {
    display: Display,
    config: Config,
    context: PossiblyCurrentContext,
    gl: glow::Context,
    progress: glow::UniformLocation,
}

pub struct Target {
    surface: Surface<WindowSurface>,
    size: (u32, u32),
}

impl Target {
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

impl Gl {
    pub fn new(wl_display: *mut c_void) -> Result<Self, String> {
        let handle = WaylandDisplayHandle::new(NonNull::new(wl_display).ok_or("no wl_display")?);
        let display = unsafe { Display::new(RawDisplayHandle::Wayland(handle)) }
            .map_err(|err| format!("EGL: {err}"))?;
        let template = ConfigTemplateBuilder::new().with_api(Api::GLES3).build();
        let config = unsafe { display.find_configs(template) }
            .map_err(|err| format!("EGL configs: {err}"))?
            .min_by_key(|config| config.alpha_size())
            .ok_or("no GLES 3 config")?;
        let attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(None);
        let context = unsafe { display.create_context(&config, &attributes) }
            .and_then(|context| context.make_current_surfaceless())
            .map_err(|err| format!("GLES 3 context: {err}"))?;
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name))
        };
        let progress = unsafe { program(&gl)? };
        Ok(Self {
            display,
            config,
            context,
            gl,
            progress,
        })
    }

    pub fn target(&self, wl_surface: *mut c_void, size: (u32, u32)) -> Result<Target, String> {
        let handle = WaylandWindowHandle::new(NonNull::new(wl_surface).ok_or("no wl_surface")?);
        let (w, h) = nonzero(size)?;
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            RawWindowHandle::Wayland(handle),
            w,
            h,
        );
        let surface = unsafe {
            self.display
                .create_window_surface(&self.config, &attributes)
        }
        .map_err(|err| format!("EGL surface: {err}"))?;
        self.context
            .make_current(&surface)
            .map_err(|err| err.to_string())?;
        surface
            .set_swap_interval(&self.context, SwapInterval::DontWait)
            .map_err(|err| err.to_string())?;
        Ok(Target { surface, size })
    }

    pub fn resize(&self, target: &mut Target, size: (u32, u32)) -> Result<(), String> {
        let (w, h) = nonzero(size)?;
        target.surface.resize(&self.context, w, h);
        target.size = size;
        Ok(())
    }

    pub fn upload(&self, target: &Target, pixels: &Pixels) -> Result<Texture, String> {
        self.context
            .make_current(&target.surface)
            .map_err(|err| err.to_string())?;
        let gl = &self.gl;
        unsafe {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            gl.tex_storage_2d(
                glow::TEXTURE_2D,
                1,
                glow::RGBA8,
                pixels.width as i32,
                pixels.height as i32,
            );
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                pixels.width as i32,
                pixels.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&pixels.rgba)),
            );
            Ok(texture)
        }
    }

    pub fn delete(&self, texture: Texture) {
        unsafe { self.gl.delete_texture(texture) }
    }

    pub fn draw(
        &self,
        target: &Target,
        before: Option<Texture>,
        after: Texture,
        progress: f32,
    ) -> Result<(), String> {
        self.context
            .make_current(&target.surface)
            .map_err(|err| err.to_string())?;
        let gl = &self.gl;
        unsafe {
            gl.viewport(0, 0, target.size.0 as i32, target.size.1 as i32);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(before.unwrap_or(after)));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(after));
            gl.uniform_1_f32(Some(&self.progress), progress);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        target
            .surface
            .swap_buffers(&self.context)
            .map_err(|err| err.to_string())
    }
}

unsafe fn program(gl: &glow::Context) -> Result<glow::UniformLocation, String> {
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(gl.get_program_info_log(program));
        }
        gl.use_program(Some(program));
        let vao = gl.create_vertex_array()?;
        gl.bind_vertex_array(Some(vao));
        for (name, unit) in [("before", 0), ("after", 1)] {
            let location = gl.get_uniform_location(program, name);
            gl.uniform_1_i32(location.as_ref(), unit);
        }
        gl.get_uniform_location(program, "progress")
            .ok_or_else(|| "no progress uniform".into())
    }
}

fn nonzero((w, h): (u32, u32)) -> Result<(NonZeroU32, NonZeroU32), String> {
    NonZeroU32::new(w)
        .zip(NonZeroU32::new(h))
        .ok_or_else(|| format!("empty surface {w}x{h}"))
}
