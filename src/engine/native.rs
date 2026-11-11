use apolline_scene::Prepared;

use super::{App, Loading, Picture, Scene, SceneShader, shading};
use crate::protocol::Fit;
use crate::{gl, picture};

pub(super) enum Native {
    Still(gl::Texture, picture::Pixels),
    Moving(Box<Scene>, picture::Pixels),
}

impl App {
    pub(super) fn native_scene(
        &self,
        index: usize,
        prepared: &Prepared,
        loading: &Loading,
    ) -> Result<Native, String> {
        let size = loading.size;
        let fit = loading.show.fit;
        picture::pixel_bytes(size.0, size.1)?;
        let mut renderer = self.gl.compile_native(prepared)?;
        renderer.follow(self.screens[index].pointer, 0.0);
        // A still scene keeps the time of day it was drawn at.
        renderer.set_daytime(shading::daytime());
        // Introspection can find a time declaration whose value the GPU optimizes away. A
        // scene that follows the pointer or listens keeps moving with it.
        let needs = renderer.needs();
        if !renderer.animated() && !needs.pointer && !needs.audio {
            return self
                .gl
                .still_scene(renderer, size, fit)
                .map(|(texture, pixels)| Native::Still(texture, pixels));
        }
        // Drawn as the part of it the monitors show: each takes its share of that.
        let budget = {
            let (w, h) = self.shader_size(size);
            f64::from(w) * f64::from(h)
        };
        let canvas_size = picture::shared_canvas(renderer.size(), [(fit, size)], budget);
        let canvas = match self.gl.canvas(canvas_size) {
            Ok(canvas) => canvas,
            Err(error) => {
                self.gl.drop_native(renderer);
                return Err(error);
            }
        };
        let clock = self.screens[index]
            .shown
            .as_ref()
            .filter(|shown| shown.show.source == loading.show.source)
            .and_then(|shown| match &shown.picture {
                Picture::Scene(key) => self.scenes.get(key).map(|shared| shared.scene.clock.fork()),
                _ => None,
            })
            .unwrap_or_default();
        let mut scene = Scene {
            shader: SceneShader::Native(Box::new(renderer), Fit::Cover),
            canvas,
            clock,
            timer: None,
            followed: None,
        };
        let result = shading::advance(
            &self.gl,
            self.effective_fps(),
            self.accent,
            1.0,
            true,
            &mut scene,
        )
        .and_then(|()| self.gl.scene_poster(&scene.canvas));
        match result {
            Ok(pixels) => Ok(Native::Moving(Box::new(scene), pixels)),
            Err(error) => {
                self.drop_scene(scene);
                Err(error)
            }
        }
    }
}
