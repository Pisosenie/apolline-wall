use apolline_scene::{Prepared, Renderer, render};
use glow::HasContext;

use super::{Canvas, Gl, Texture};
use crate::picture;
use crate::protocol::Fit;

impl Gl {
    pub fn compile_native(&self, scene: &Prepared) -> Result<Renderer, String> {
        self.current()?;
        let mut renderer = unsafe { Renderer::new(&self.gl, &scene.scene, &scene.layers) }
            .map_err(|err| err.to_string())?;
        // Each screen runs the scene's scripts on its own.
        match scene.stage() {
            Ok(stage) => {
                if let Some(stage) = stage {
                    renderer.script(stage);
                }
                Ok(renderer)
            }
            Err(err) => {
                self.drop_native(renderer);
                Err(err.to_string())
            }
        }
    }

    pub fn drop_native(&self, renderer: Renderer) {
        unsafe { renderer.delete(&self.gl) };
    }

    /// Runs a frame of the scene's scripts, then draws it.
    pub fn run_native(
        &self,
        renderer: &mut Renderer,
        canvas: &Canvas,
        fit: Fit,
        time: f32,
        delta: f32,
    ) -> Result<(), String> {
        self.current()?;
        unsafe { renderer.animate(&self.gl, time, delta) };
        let fit = match fit {
            Fit::Cover => render::Fit::Cover,
            // Scenes have no blur yet: whole, with plain edges.
            Fit::Contain | Fit::Blur => render::Fit::Contain,
        };
        unsafe {
            crate::gl::clear_errors(&self.gl);
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(canvas.framebuffer));
            renderer.draw_timed(&self.gl, canvas.size, fit, time, delta);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            let error = self.gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("drawing the native scene failed (GL {error:#x})"));
            }
        }
        Ok(())
    }

    /// Still scenes are flattened once, then cost exactly as much as a static wallpaper.
    pub fn still_scene(
        &self,
        renderer: Renderer,
        size: (u32, u32),
        fit: Fit,
    ) -> Result<(Texture, picture::Pixels), String> {
        self.current()?;

        let canvas = match self.canvas(size) {
            Ok(canvas) => canvas,
            Err(err) => {
                unsafe { renderer.delete(&self.gl) };
                return Err(err);
            }
        };
        let fit = match fit {
            Fit::Cover => render::Fit::Cover,
            // Scenes have no blur yet: whole, with plain edges.
            Fit::Contain | Fit::Blur => render::Fit::Contain,
        };
        let error = unsafe {
            crate::gl::clear_errors(&self.gl);
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(canvas.framebuffer));
            renderer.draw_fitted(&self.gl, size, fit);
            renderer.delete(&self.gl);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.gl.get_error()
        };
        if error != glow::NO_ERROR {
            self.free(canvas);
            return Err(format!("drawing the scene failed (GL {error:#x})"));
        }
        let pixels = match self.scene_poster(&canvas) {
            Ok(pixels) => pixels,
            Err(error) => {
                self.free(canvas);
                return Err(error);
            }
        };
        unsafe { self.gl.delete_framebuffer(canvas.framebuffer) };
        Ok((canvas.texture, pixels))
    }
    pub fn scene_poster(&self, canvas: &Canvas) -> Result<picture::Pixels, String> {
        let size = canvas.size;
        let scale = (640.0 / size.0 as f64).min(360.0 / size.1 as f64).min(1.0);
        let small = (
            (size.0 as f64 * scale).round().max(1.0) as u32,
            (size.1 as f64 * scale).round().max(1.0) as u32,
        );
        let preview = self.canvas(small)?;
        unsafe {
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, Some(canvas.framebuffer));
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(preview.framebuffer));
            self.gl.blit_framebuffer(
                0,
                0,
                size.0 as i32,
                size.1 as i32,
                0,
                0,
                small.0 as i32,
                small.1 as i32,
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        let pixels = self.read(&preview);
        self.free(preview);
        let rgba = pixels?;
        Ok(picture::Pixels {
            width: small.0,
            height: small.1,
            rgba,
        })
    }
}
