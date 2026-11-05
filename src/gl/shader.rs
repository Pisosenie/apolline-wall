use glow::HasContext;

use super::{Canvas, Gl, link};

/// Shadertoy's inputs plus the shell's accent; `#line 1` keeps error lines pointing into the file.
const HEADER: &str = "#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;
uniform vec3 iResolution;
uniform float iTime;
uniform float iTimeDelta;
uniform float iFrameRate;
uniform int iFrame;
uniform vec4 iMouse;
uniform vec4 iDate;
uniform float iSampleRate;
uniform vec3 iChannelResolution[4];
uniform float iChannelTime[4];
uniform sampler2D iChannel0;
uniform sampler2D iChannel1;
uniform sampler2D iChannel2;
uniform sampler2D iChannel3;
uniform vec3 iAccent;
out vec4 apollineColor;
#line 1
";

const FOOTER: &str = "
void main() {
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
    mainImage(color, gl_FragCoord.xy);
    apollineColor = vec4(color.rgb, 1.0);
}
";

pub struct Shader {
    program: glow::Program,
    resolution: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    delta: Option<glow::UniformLocation>,
    rate: Option<glow::UniformLocation>,
    frame: Option<glow::UniformLocation>,
    date: Option<glow::UniformLocation>,
    accent: Option<glow::UniformLocation>,
}

pub struct Inputs {
    pub time: f32,
    pub delta: f32,
    pub fps: f32,
    pub frame: i32,
    pub date: [f32; 4],
    pub accent: [f32; 3],
}

impl Gl {
    pub fn shader(&self, source: &str) -> Result<Shader, String> {
        self.current()?;
        let gl = &self.gl;
        unsafe {
            let program = link(gl, &format!("{HEADER}{source}{FOOTER}"))?;
            let at = |name: &str| gl.get_uniform_location(program, name);
            gl.use_program(Some(program));
            for unit in 0..4 {
                gl.uniform_1_i32(at(&format!("iChannel{unit}")).as_ref(), unit);
            }
            let noise = [super::NOISE.0 as f32, super::NOISE.1 as f32, 1.0].repeat(4);
            gl.uniform_3_f32_slice(at("iChannelResolution").as_ref(), &noise);
            gl.uniform_4_f32(at("iMouse").as_ref(), 0.0, 0.0, 0.0, 0.0);
            gl.uniform_1_f32(at("iSampleRate").as_ref(), 44100.0);
            Ok(Shader {
                program,
                resolution: at("iResolution"),
                time: at("iTime"),
                delta: at("iTimeDelta"),
                rate: at("iFrameRate"),
                frame: at("iFrame"),
                date: at("iDate"),
                accent: at("iAccent"),
            })
        }
    }

    pub fn run(&self, shader: &Shader, canvas: &Canvas, inputs: &Inputs) -> Result<(), String> {
        self.current()?;
        let gl = &self.gl;
        let (w, h) = canvas.size;
        unsafe {
            crate::gl::clear_errors(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(canvas.framebuffer));
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.viewport(0, 0, w as i32, h as i32);
            gl.use_program(Some(shader.program));
            gl.bind_vertex_array(Some(self.program.vao));
            for unit in 0..4 {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.noise));
            }
            gl.uniform_3_f32(shader.resolution.as_ref(), w as f32, h as f32, 1.0);
            gl.uniform_1_f32(shader.time.as_ref(), inputs.time);
            gl.uniform_1_f32(shader.delta.as_ref(), inputs.delta);
            gl.uniform_1_f32(shader.rate.as_ref(), inputs.fps);
            gl.uniform_1_i32(shader.frame.as_ref(), inputs.frame);
            gl.uniform_4_f32_slice(shader.date.as_ref(), &inputs.date);
            gl.uniform_3_f32_slice(shader.accent.as_ref(), &inputs.accent);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("drawing the shader failed (GL {error:#x})"));
            }
        }
        Ok(())
    }

    pub fn forget(&self, shader: Shader) {
        unsafe { self.gl.delete_program(shader.program) }
    }

    /// Top row first, as image files expect.
    pub fn read(&self, canvas: &Canvas) -> Result<Vec<u8>, String> {
        self.current()?;
        let (w, h) = (canvas.size.0 as usize, canvas.size.1 as usize);
        let mut rgba = vec![0; w * h * 4];
        let gl = &self.gl;
        unsafe {
            crate::gl::clear_errors(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(canvas.framebuffer));
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 0);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut rgba)),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("reading a poster failed (GL {error:#x})"));
            }
        }
        let row = w * 4;
        for y in 0..h / 2 {
            let (top, bottom) = rgba.split_at_mut((h - 1 - y) * row);
            top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
        Ok(rgba)
    }
}
