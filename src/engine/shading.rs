mod source;

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};

use super::{App, Picture, Scene, SceneShader};
use crate::gl::{self, Gl};
use crate::picture;
use crate::protocol::{self, Event, Limits, Uniforms};

pub(super) const DEFAULT_FPS: u32 = 30;
pub(super) const DEFAULT_ACCENT: [f32; 3] = [134.0 / 255.0, 189.0 / 255.0, 240.0 / 255.0];

impl App {
    pub(super) fn start_scene(&mut self, index: usize) {
        let Some(loading) = self.screens[index].loading.take() else {
            return;
        };
        let output = self.screens[index].name.clone();
        let source = loading.show.source.clone();
        match self.scene(&source, loading.size) {
            Ok(scene) => {
                // Full size: the shell shows a shader's poster on the lock screen.
                let pixels = self.gl.read(&scene.canvas).map(|rgba| picture::Pixels {
                    width: scene.canvas.size.0,
                    height: scene.canvas.size.1,
                    rgba,
                });
                let generation = loading.generation;
                self.present(index, Picture::Shader(Box::new(scene)), loading);
                self.queue_poster(output, source, generation, pixels);
            }
            Err(message) => {
                self.give_up(index, loading.show);
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(source),
                    message,
                });
            }
        }
    }

    fn scene(&self, source: &Path, size: (u32, u32)) -> Result<Scene, String> {
        let text = self::source::read(source)?;
        let shader = self
            .gl
            .shader(&text)
            .map_err(|log| format!("{}: {log}", source.display()))?;
        let canvas = match self.gl.canvas(self.shader_size(size)) {
            Ok(canvas) => canvas,
            Err(message) => {
                self.gl.forget(shader);
                return Err(message);
            }
        };
        let mut scene = Scene {
            shader: SceneShader::Toy(shader),
            canvas,
            clock: crate::animation::Clock::default(),
            timer: None,
        };
        if let Err(message) = advance(
            &self.gl,
            self.effective_fps(),
            self.accent,
            1.0,
            true,
            &mut scene,
        ) {
            self.drop_scene(scene);
            return Err(message);
        }
        Ok(scene)
    }

    pub(super) fn drop_scene(&self, scene: Scene) {
        if let Some(token) = scene.timer {
            self.handle.remove(token);
        }
        match scene.shader {
            SceneShader::Toy(shader) => self.gl.forget(shader),
            SceneShader::Native(renderer, _) => self.gl.drop_native(renderer),
        }
        self.gl.free(scene.canvas);
    }

    /// Called when the compositor wants a frame: draws now if the frame is due, or sets a timer.
    pub(super) fn tick_scene(&mut self, index: usize) {
        let fps = self.effective_fps();
        let id = self.screens[index].id;
        let Some(shown) = self.screens[index].shown.as_mut() else {
            return;
        };
        let generation = shown.generation;
        let Picture::Shader(scene) = &mut shown.picture else {
            return;
        };
        let wait = scene.clock.wait(Instant::now(), fps);
        if wait.is_zero() {
            self.step(index);
        } else if scene.timer.is_none() {
            let timer = Timer::from_duration(wait);
            scene.timer = self
                .handle
                .insert_source(timer, move |_, _, app: &mut App| {
                    app.scene_due(id, generation);
                    TimeoutAction::Drop
                })
                .ok();
        }
    }

    fn scene_due(&mut self, id: u64, generation: u64) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let Some(Picture::Shader(scene)) = self.screens[index]
            .shown
            .as_mut()
            .filter(|shown| shown.generation == generation)
            .map(|shown| &mut shown.picture)
        else {
            return;
        };
        scene.timer = None;
        let screen = &self.screens[index];
        if !screen.frame_pending && !self.is_paused(&screen.name) {
            self.step(index);
        }
    }

    fn step(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let (fps, accent) = (self.effective_fps(), self.accent);
        let screen = &mut self.screens[index];
        let Some(shown) = screen.shown.as_mut() else {
            return;
        };
        let speed = shown.show.look.unwrap_or_default().speed;
        let Picture::Shader(scene) = &mut shown.picture else {
            return;
        };
        if let Some(token) = scene.timer.take() {
            self.handle.remove(token);
        }
        let result = advance(&self.gl, fps, accent, speed, !paused, scene);
        if let Err(message) = result {
            let show = shown.show.clone();
            let output = screen.name.clone();
            if let Some(shown) = self.screens[index].shown.take()
                && let Picture::Shader(scene) = shown.picture
            {
                self.drop_scene(*scene);
            }
            if let Some(fade) = self.screens[index].fade.take() {
                self.gl.delete(fade.before.texture);
            }
            self.give_up(index, show.clone());
            protocol::emit(&Event::Error {
                output: Some(output),
                source: Some(show.source),
                message,
            });
            protocol::emit(&self.status());
            return;
        }
        self.draw(index);
    }

    /// A paused shader keeps its clock still, so it resumes where it stopped.
    pub(super) fn pause_scenes(&mut self) {
        for index in 0..self.screens.len() {
            let screen = &self.screens[index];
            let paused = self.is_paused(&screen.name);
            let waiting = screen.frame_pending;
            let Some(Picture::Shader(scene)) = self.screens[index]
                .shown
                .as_mut()
                .map(|shown| &mut shown.picture)
            else {
                continue;
            };
            if paused {
                scene.clock.pause();
                if let Some(token) = scene.timer.take() {
                    self.handle.remove(token);
                }
            } else if !waiting {
                self.step(index);
            }
        }
    }

    pub(super) fn limit(&mut self, limits: Limits) {
        let changed_fps = limits.fps.is_some_and(|fps| fps != self.fps);
        if let Some(fps) = limits.fps {
            self.fps = fps;
        }
        if let Some(scale) = limits.scale {
            self.scale = Some(scale);
            for index in 0..self.screens.len() {
                self.rescale_scene(index);
            }
        }
        if changed_fps {
            self.pause_scenes();
        }
    }

    pub(super) fn rescale_scene(&mut self, index: usize) {
        let Some(size) = self.screens[index].target.as_ref().map(|t| t.size()) else {
            return;
        };
        let wanted = self.shader_size(size);
        let screen = &mut self.screens[index];
        let Some(Picture::Shader(scene)) = screen.shown.as_mut().map(|shown| &mut shown.picture)
        else {
            return;
        };
        if scene.canvas.size == wanted {
            return;
        }
        match self.gl.canvas(wanted) {
            Ok(canvas) => {
                let old = std::mem::replace(&mut scene.canvas, canvas);
                self.gl.free(old);
            }
            Err(message) => {
                protocol::emit(&Event::Error {
                    output: Some(screen.name.clone()),
                    source: None,
                    message,
                });
                return;
            }
        }
        self.step(index);
    }

    /// A new accent shows at once, on paused shaders too.
    pub(super) fn tint(&mut self, uniforms: Uniforms) {
        let Some(accent) = uniforms.accent else {
            return;
        };
        self.accent = accent;
        for index in 0..self.screens.len() {
            let shader = matches!(
                self.screens[index]
                    .shown
                    .as_ref()
                    .map(|shown| &shown.picture),
                Some(Picture::Shader(scene)) if matches!(scene.shader, SceneShader::Toy(_))
            );
            if shader {
                self.step(index);
            }
        }
    }
}

pub(super) fn advance(
    gl: &Gl,
    fps: u32,
    accent: [f32; 3],
    speed: f32,
    running: bool,
    scene: &mut Scene,
) -> Result<(), String> {
    let frame = scene.clock.tick(Instant::now(), speed, running, fps);
    match &mut scene.shader {
        SceneShader::Toy(shader) => {
            let inputs = gl::Inputs {
                time: frame.time,
                delta: frame.delta,
                fps: fps as f32,
                frame: frame.number,
                date: date(),
                accent,
            };
            gl.run(shader, &scene.canvas, &inputs)?;
        }
        SceneShader::Native(renderer, fit) => {
            renderer.set_daytime(daytime());
            gl.run_native(renderer, &scene.canvas, *fit, frame.time, frame.delta)?
        }
    }
    scene.clock.rendered();
    Ok(())
}

unsafe extern "C" {
    fn tzset();
}

pub(super) fn local_time_zone() {
    unsafe { tzset() };
}

/// How far through the local day it is, 0 to 1, for scenes that change with it.
pub(super) fn daytime() -> f32 {
    date()[3] / 86_400.0
}

/// Shadertoy's iDate: year, month from zero, day, and seconds since local midnight.
fn date() -> [f32; 4] {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as libc::time_t;
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut local) };
    let of_day = (local.tm_hour * 3600 + local.tm_min * 60 + local.tm_sec) as f32
        + now.subsec_millis() as f32 / 1000.0;
    [
        (local.tm_year + 1900) as f32,
        local.tm_mon as f32,
        local.tm_mday as f32,
        of_day,
    ]
}
