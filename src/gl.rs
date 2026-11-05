mod scenery;
mod shader;

use std::num::NonZeroU32;

use glow::HasContext;
use glutin::api::egl::config::Config;
use glutin::api::egl::context::PossiblyCurrentContext;
use glutin::api::egl::display::Display;
use glutin::api::egl::surface::Surface;
use glutin::config::{Api, ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentGlContext, Version};
use glutin::display::GlDisplay;
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::picture::{self, Pixels, Uv};
use crate::protocol::Look;

pub use glow::Texture;

/// Drops error flags earlier calls left behind on the shared context, libmpv's and the
/// scene renderer's included, so a check after a draw only sees what that draw did.
pub fn clear_errors(gl: &glow::Context) {
    for _ in 0..16 {
        if unsafe { gl.get_error() } == glow::NO_ERROR {
            break;
        }
    }
}
pub use shader::{Inputs, Shader};

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
uniform vec4 before_uv;
uniform vec4 after_uv;
uniform vec4 before_tone;
uniform vec4 after_tone;
uniform float progress;
in vec2 uv;
out vec4 color;
// tone: brightness, contrast around mid-grey, saturation.
vec3 pick(sampler2D picture, vec4 placement, vec4 tone) {
    vec2 p = uv * placement.xy + placement.zw;
    if (any(lessThan(p, vec2(0.0))) || any(greaterThan(p, vec2(1.0)))) {
        return vec3(0.0);
    }
    vec3 c = (texture(picture, p).rgb - 0.5) * tone.y + 0.5 + tone.x;
    float grey = dot(c, vec3(0.2126, 0.7152, 0.0722));
    return clamp(mix(vec3(grey), c, tone.z), 0.0, 1.0);
}
void main() {
    vec3 a = pick(before, before_uv, before_tone);
    vec3 b = pick(after, after_uv, after_tone);
    color = vec4(mix(a, b, progress), 1.0);
}";

pub struct Gl {
    display: Display,
    config: Config,
    context: PossiblyCurrentContext,
    gl: glow::Context,
    program: Program,
    noise: Texture,
}

struct Program {
    id: glow::Program,
    vao: glow::VertexArray,
    before_uv: glow::UniformLocation,
    after_uv: glow::UniformLocation,
    before_tone: glow::UniformLocation,
    after_tone: glow::UniformLocation,
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

#[derive(Clone, Copy)]
pub struct Layer {
    pub texture: Texture,
    pub uv: Uv,
    pub tone: [f32; 4],
}

impl Layer {
    pub fn new(texture: Texture, uv: Uv, look: Option<Look>) -> Self {
        let look = look.unwrap_or_default();
        Layer {
            texture,
            uv: if look.flip { picture::flipped(uv) } else { uv },
            tone: picture::tone(&look),
        }
    }
}

pub struct Canvas {
    framebuffer: glow::Framebuffer,
    pub texture: Texture,
    pub size: (u32, u32),
}

impl Canvas {
    pub fn framebuffer(&self) -> u32 {
        self.framebuffer.0.get()
    }
}

impl Gl {
    /// A GLES 3 context on the session's display, Wayland's or X11's.
    pub fn new(handle: RawDisplayHandle) -> Result<Self, String> {
        let display = unsafe { Display::new(handle) }.map_err(|err| format!("EGL: {err}"))?;
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
        let program = unsafe { program(&gl)? };
        let noise = unsafe { texture(&gl, NOISE, Some(&noise()), glow::REPEAT)? };
        Ok(Self {
            display,
            config,
            context,
            gl,
            program,
            noise,
        })
    }

    pub fn display(&self) -> &Display {
        &self.display
    }

    pub fn current(&self) -> Result<(), String> {
        self.context
            .make_current_surfaceless()
            .map_err(|err| err.to_string())
    }

    pub fn target(&self, window: RawWindowHandle, size: (u32, u32)) -> Result<Target, String> {
        let (w, h) = nonzero(size)?;
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(window, w, h);
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

    pub fn upload(&self, pixels: &Pixels) -> Result<Texture, String> {
        self.current()?;
        let size = (pixels.width, pixels.height);
        unsafe { texture(&self.gl, size, Some(&pixels.rgba), glow::CLAMP_TO_EDGE) }
    }

    pub fn canvas(&self, size: (u32, u32)) -> Result<Canvas, String> {
        self.current()?;
        picture::pixel_bytes(size.0, size.1)?;
        let gl = &self.gl;
        unsafe {
            let texture = texture(gl, size, None, glow::CLAMP_TO_EDGE)?;
            let framebuffer = match gl.create_framebuffer() {
                Ok(framebuffer) => framebuffer,
                Err(err) => {
                    gl.delete_texture(texture);
                    return Err(err);
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(framebuffer);
                gl.delete_texture(texture);
                return Err(format!("framebuffer {}x{} is incomplete", size.0, size.1));
            }
            Ok(Canvas {
                framebuffer,
                texture,
                size,
            })
        }
    }

    pub fn snapshot(&self, canvas: &Canvas) -> Result<Texture, String> {
        let snapshot = self.canvas(canvas.size)?;
        let (w, h) = (canvas.size.0 as i32, canvas.size.1 as i32);
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(canvas.framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(snapshot.framebuffer));
            gl.blit_framebuffer(
                0,
                0,
                w,
                h,
                0,
                0,
                w,
                h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(snapshot.framebuffer);
        }
        Ok(snapshot.texture)
    }

    pub fn delete(&self, texture: Texture) {
        unsafe { self.gl.delete_texture(texture) }
    }

    pub fn free(&self, canvas: Canvas) {
        unsafe {
            self.gl.delete_framebuffer(canvas.framebuffer);
            self.gl.delete_texture(canvas.texture);
        }
    }

    pub fn draw(
        &self,
        target: &Target,
        before: Option<Layer>,
        after: Layer,
        progress: f32,
    ) -> Result<(), String> {
        self.context
            .make_current(&target.surface)
            .map_err(|err| err.to_string())?;
        let gl = &self.gl;
        let before = before.unwrap_or(after);
        let program = &self.program;
        // libmpv draws with the same context and leaves its own state behind.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(program.id));
            gl.bind_vertex_array(Some(program.vao));
            gl.viewport(0, 0, target.size.0 as i32, target.size.1 as i32);
            for (unit, layer) in [(glow::TEXTURE0, before), (glow::TEXTURE1, after)] {
                gl.active_texture(unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(layer.texture));
            }
            gl.uniform_4_f32_slice(Some(&program.before_uv), &before.uv);
            gl.uniform_4_f32_slice(Some(&program.after_uv), &after.uv);
            gl.uniform_4_f32_slice(Some(&program.before_tone), &before.tone);
            gl.uniform_4_f32_slice(Some(&program.after_tone), &after.tone);
            gl.uniform_1_f32(Some(&program.progress), progress);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        target
            .surface
            .swap_buffers(&self.context)
            .map_err(|err| err.to_string())
    }
}

unsafe fn texture(
    gl: &glow::Context,
    (w, h): (u32, u32),
    rgba: Option<&[u8]>,
    wrap: u32,
) -> Result<Texture, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, wrap),
            (glow::TEXTURE_WRAP_T, wrap),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
        }
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        if let Some(rgba) = rgba {
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(rgba)),
            );
        }
        Ok(texture)
    }
}

unsafe fn link(gl: &glow::Context, fragment: &str) -> Result<glow::Program, String> {
    unsafe {
        let id = gl.create_program()?;
        let mut shaders = Vec::new();
        let mut failed = None;
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                failed = Some(gl.get_shader_info_log(shader));
            }
            gl.attach_shader(id, shader);
            shaders.push(shader);
        }
        if failed.is_none() {
            gl.link_program(id);
            if !gl.get_program_link_status(id) {
                failed = Some(gl.get_program_info_log(id));
            }
        }
        for shader in shaders {
            gl.detach_shader(id, shader);
            gl.delete_shader(shader);
        }
        match failed {
            Some(log) => {
                gl.delete_program(id);
                Err(log.trim().to_owned())
            }
            None => Ok(id),
        }
    }
}

unsafe fn program(gl: &glow::Context) -> Result<Program, String> {
    unsafe {
        let id = link(gl, FRAGMENT)?;
        gl.use_program(Some(id));
        for (name, unit) in [("before", 0), ("after", 1)] {
            let location = gl.get_uniform_location(id, name);
            gl.uniform_1_i32(location.as_ref(), unit);
        }
        let uniform = |name: &str| {
            gl.get_uniform_location(id, name)
                .ok_or_else(|| format!("no {name} uniform"))
        };
        Ok(Program {
            id,
            vao: gl.create_vertex_array()?,
            before_uv: uniform("before_uv")?,
            after_uv: uniform("after_uv")?,
            before_tone: uniform("before_tone")?,
            after_tone: uniform("after_tone")?,
            progress: uniform("progress")?,
        })
    }
}

const NOISE: (u32, u32) = (256, 256);

/// Shadertoy shaders often sample a noise texture from their channels; this one stands in for it.
fn noise() -> Vec<u8> {
    let mut state: u32 = 0x9e37_79b9;
    (0..NOISE.0 * NOISE.1 * 4)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

fn nonzero((w, h): (u32, u32)) -> Result<(NonZeroU32, NonZeroU32), String> {
    NonZeroU32::new(w)
        .zip(NonZeroU32::new(h))
        .ok_or_else(|| format!("empty surface {w}x{h}"))
}
