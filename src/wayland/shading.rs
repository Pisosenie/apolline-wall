use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput;

use super::{App, Picture, Scene, poster_path};
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
                let poster = self.poster(&scene, &source);
                self.present(index, Picture::Shader(Box::new(scene)), loading);
                protocol::emit(&Event::Ready {
                    output,
                    source,
                    poster,
                });
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
        let text = std::fs::read_to_string(source)
            .map_err(|err| format!("{}: {err}", source.display()))?;
        let shader = self
            .gl
            .shader(&text)
            .map_err(|log| format!("{}: {log}", source.display()))?;
        let canvas = match self.gl.canvas(picture::shader_size(size, self.scale)) {
            Ok(canvas) => canvas,
            Err(message) => {
                self.gl.forget(shader);
                return Err(message);
            }
        };
        let mut scene = Scene {
            shader,
            canvas,
            time: 0.0,
            last: None,
            frame: 0,
            timer: false,
        };
        if let Err(message) = advance(&self.gl, self.fps, self.accent, 1.0, &mut scene) {
            self.drop_scene(scene);
            return Err(message);
        }
        Ok(scene)
    }

    fn poster(&self, scene: &Scene, source: &Path) -> PathBuf {
        let path = poster_path(source);
        let written = self
            .gl
            .read(&scene.canvas)
            .and_then(|rgba| picture::save_png(&path, &rgba, scene.canvas.size));
        match written {
            Ok(()) => path,
            Err(message) => {
                protocol::emit(&Event::Error {
                    output: None,
                    source: Some(source.to_path_buf()),
                    message: format!("poster frame: {message}"),
                });
                source.to_path_buf()
            }
        }
    }

    pub(super) fn drop_scene(&self, scene: Scene) {
        self.gl.forget(scene.shader);
        self.gl.free(scene.canvas);
    }

    /// Called when the compositor wants a frame: draws now if the frame is due, or sets a timer.
    pub(super) fn tick_scene(&mut self, index: usize) {
        let interval = Duration::from_secs_f64(1.0 / f64::from(self.fps));
        let Some(Picture::Shader(scene)) = self.screens[index]
            .shown
            .as_mut()
            .map(|shown| &mut shown.picture)
        else {
            return;
        };
        let wait = scene.last.map_or(Duration::ZERO, |last| {
            interval.saturating_sub(last.elapsed())
        });
        if wait.is_zero() {
            self.step(index);
        } else if !scene.timer {
            scene.timer = true;
            let output = self.screens[index].output.clone();
            let timer = Timer::from_duration(wait);
            let _ = self
                .handle
                .insert_source(timer, move |_, _, app: &mut App| {
                    app.scene_due(&output);
                    TimeoutAction::Drop
                });
        }
    }

    fn scene_due(&mut self, output: &WlOutput) {
        let Some(index) = self.screens.iter().position(|s| &s.output == output) else {
            return;
        };
        let Some(Picture::Shader(scene)) = self.screens[index]
            .shown
            .as_mut()
            .map(|shown| &mut shown.picture)
        else {
            return;
        };
        scene.timer = false;
        let screen = &self.screens[index];
        if !screen.frame_pending && !self.is_paused(&screen.name) {
            self.step(index);
        }
    }

    fn step(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let (fps, accent) = (self.fps, self.accent);
        let screen = &mut self.screens[index];
        let Some(shown) = screen.shown.as_mut() else {
            return;
        };
        let speed = shown.show.look.unwrap_or_default().speed;
        let Picture::Shader(scene) = &mut shown.picture else {
            return;
        };
        let result = advance(&self.gl, fps, accent, speed, scene);
        if paused {
            scene.last = None;
        }
        if let Err(message) = result {
            protocol::emit(&Event::Error {
                output: Some(screen.name.clone()),
                source: None,
                message,
            });
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
                scene.last = None;
            } else if !waiting {
                self.step(index);
            }
        }
    }

    pub(super) fn limit(&mut self, limits: Limits) {
        if let Some(fps) = limits.fps {
            self.fps = fps;
        }
        if let Some(scale) = limits.scale {
            self.scale = Some(scale);
            for index in 0..self.screens.len() {
                self.rescale_scene(index);
            }
        }
    }

    fn rescale_scene(&mut self, index: usize) {
        let Some(size) = self.screens[index].target.as_ref().map(|t| t.size()) else {
            return;
        };
        let wanted = picture::shader_size(size, self.scale);
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
                Some(Picture::Shader(_))
            );
            if shader {
                self.step(index);
            }
        }
    }
}

fn advance(
    gl: &Gl,
    fps: u32,
    accent: [f32; 3],
    speed: f32,
    scene: &mut Scene,
) -> Result<(), String> {
    let now = Instant::now();
    let delta = scene
        .last
        .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
    scene.time += delta * speed;
    scene.last = Some(now);
    let inputs = gl::Inputs {
        time: scene.time,
        delta: delta.min(0.25),
        fps: fps as f32,
        frame: scene.frame,
        date: date(),
        accent,
    };
    gl.run(&scene.shader, &scene.canvas, &inputs)?;
    scene.frame += 1;
    Ok(())
}

unsafe extern "C" {
    fn tzset();
}

pub(super) fn local_time_zone() {
    unsafe { tzset() };
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
