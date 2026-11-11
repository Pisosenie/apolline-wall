//! A moving native scene runs once for every monitor showing it, the way a video plays once:
//! it draws into a canvas of its own aspect, and each monitor takes its part of that. The
//! scene's particles, scripts and effects cost the same however many monitors there are.

use std::path::PathBuf;
use std::time::Instant;

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{LoopHandle, RegistrationToken};

use super::shading::advance;
use super::{App, Picture, Scene, SceneShader};
use crate::gl;
use crate::picture;
use crate::protocol::{self, Event, Fit, Show};

/// A scene and its property values: monitors with the same edits share one renderer.
pub(super) type SceneKey = (PathBuf, String);

pub(super) fn key(show: &Show) -> SceneKey {
    (
        show.source.clone(),
        serde_json::to_string(&show.properties).unwrap_or_default(),
    )
}

pub(super) struct Shared {
    pub scene: Scene,
    /// Tells this scene's timers from those of one that had the same key before.
    id: u64,
}

fn due_after(
    handle: &LoopHandle<'static, App>,
    key: &SceneKey,
    id: u64,
    wait: std::time::Duration,
) -> Option<RegistrationToken> {
    let key = key.clone();
    handle
        .insert_source(Timer::from_duration(wait), move |_, _, app: &mut App| {
            app.shared_due(&key, id);
            TimeoutAction::Drop
        })
        .ok()
}

pub(super) fn scene_size(scene: &Scene) -> (f32, f32) {
    match &scene.shader {
        SceneShader::Native(renderer, _) => renderer.size(),
        SceneShader::Toy(_) => (scene.canvas.size.0 as f32, scene.canvas.size.1 as f32),
    }
}

impl App {
    pub(super) fn share_scene(&mut self, key: SceneKey, scene: Scene) {
        self.generation += 1;
        let shared = Shared {
            scene,
            id: self.generation,
        };
        if let Some(old) = self.scenes.insert(key, shared) {
            self.drop_scene(old.scene);
        }
    }

    pub(super) fn scene_users(&self, key: &SceneKey) -> Vec<usize> {
        self.screens
            .iter()
            .enumerate()
            .filter(|(_, screen)| {
                matches!(
                    screen.shown.as_ref().map(|shown| &shown.picture),
                    Some(Picture::Scene(shown)) if shown == key
                )
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The part of the shared canvas screen `index` shows.
    pub(super) fn scene_layer(&self, index: usize, key: &SceneKey) -> Option<gl::Layer> {
        let screen = &self.screens[index];
        let (shown, target) = (screen.shown.as_ref()?, screen.target.as_ref()?);
        let canvas = &self.scenes.get(key)?.scene.canvas;
        Some(
            gl::Layer::fitted(
                canvas.texture,
                shown.show.fit,
                canvas.size,
                target.size(),
                shown.show.look,
            )
            .rendered(),
        )
    }

    /// What a monitor fading away from the scene keeps of it.
    pub(super) fn scene_snapshot(
        &self,
        index: usize,
        key: &SceneKey,
        show: &Show,
    ) -> Option<gl::Layer> {
        let canvas = &self.scenes.get(key)?.scene.canvas;
        let target = self.screens[index].target.as_ref()?;
        Some(
            gl::Layer::fitted(
                self.gl.snapshot(canvas).ok()?,
                show.fit,
                canvas.size,
                target.size(),
                show.look,
            )
            .rendered(),
        )
    }

    /// Sizes each shared canvas for the monitors showing it now.
    pub(super) fn fit_scenes(&mut self) {
        let keys: Vec<SceneKey> = self.scenes.keys().cloned().collect();
        for key in keys {
            self.fit_scene(&key);
        }
    }

    pub(super) fn fit_scene(&mut self, key: &SceneKey) {
        let targets: Vec<(Fit, (u32, u32))> = self
            .scene_users(key)
            .into_iter()
            .filter_map(|index| {
                let screen = &self.screens[index];
                Some((
                    screen.shown.as_ref()?.show.fit,
                    screen.target.as_ref()?.size(),
                ))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let budget: f64 = targets
            .iter()
            .map(|(_, size)| {
                let (w, h) = self.shader_size(*size);
                f64::from(w) * f64::from(h)
            })
            .sum();
        let Some(shared) = self.scenes.get_mut(key) else {
            return;
        };
        let wanted = picture::shared_canvas(scene_size(&shared.scene), targets, budget);
        if shared.scene.canvas.size == wanted {
            return;
        }
        match self.gl.canvas(wanted) {
            Ok(canvas) => {
                let old = std::mem::replace(&mut shared.scene.canvas, canvas);
                self.gl.free(old);
            }
            Err(message) => {
                protocol::emit(&Event::Error {
                    output: None,
                    source: Some(key.0.clone()),
                    message,
                });
                return;
            }
        }
        // A new canvas starts empty: it is drawn now, even for a scene the pointer hasn't moved.
        self.draw_shared(key, true);
    }

    pub(super) fn close_unused_scenes(&mut self) {
        let unused: Vec<SceneKey> = self
            .scenes
            .keys()
            .filter(|key| self.scene_users(key).is_empty())
            .cloned()
            .collect();
        for key in unused {
            if let Some(shared) = self.scenes.remove(&key) {
                self.drop_scene(shared.scene);
            }
        }
    }

    /// Called when a monitor showing the scene wants a frame: draws now if one is due, or
    /// sets a timer.
    pub(super) fn tick_shared(&mut self, key: &SceneKey) {
        let fps = self.effective_fps();
        let Some(shared) = self.scenes.get_mut(key) else {
            return;
        };
        let wait = shared.scene.clock.wait(Instant::now(), fps);
        if wait.is_zero() {
            self.step_shared(key);
        } else if shared.scene.timer.is_none() {
            shared.scene.timer = due_after(&self.handle, key, shared.id, wait);
        }
    }

    fn shared_due(&mut self, key: &SceneKey, id: u64) {
        let Some(shared) = self.scenes.get_mut(key).filter(|shared| shared.id == id) else {
            return;
        };
        shared.scene.timer = None;
        let ready = self.scene_users(key).into_iter().any(|index| {
            let screen = &self.screens[index];
            !screen.frame_pending && !self.is_paused(&screen.name)
        });
        if ready {
            self.step_shared(key);
        }
    }

    pub(super) fn step_shared(&mut self, key: &SceneKey) {
        self.draw_shared(key, false);
    }

    fn draw_shared(&mut self, key: &SceneKey, force: bool) {
        let users = self.scene_users(key);
        if users.is_empty() {
            return;
        }
        let paused = users
            .iter()
            .all(|&index| self.is_paused(&self.screens[index].name));
        let follows = self.scenes.get(key).is_some_and(|shared| {
            matches!(&shared.scene.shader, SceneShader::Native(renderer, _) if renderer.needs().pointer)
        });
        let pointer = if follows {
            self.shared_pointer(key, &users)
        } else {
            [0.0; 2]
        };
        // The first monitor's pace leads; the scene can only run at one.
        let speed = self.screens[users[0]]
            .shown
            .as_ref()
            .map_or(1.0, |shown| shown.show.look.unwrap_or_default().speed);
        let (fps, accent) = (self.effective_fps(), self.accent);
        let Some(shared) = self.scenes.get_mut(key) else {
            return;
        };
        let id = shared.id;
        let scene = &mut shared.scene;
        if let Some(token) = scene.timer.take() {
            self.handle.remove(token);
        }
        if let SceneShader::Native(renderer, _) = &mut scene.shader
            && renderer.needs().pointer
        {
            let now = Instant::now();
            let seconds = scene
                .followed
                .map_or(0.0, |then| now.duration_since(then).as_secs_f32().min(0.25));
            scene.followed = Some(now);
            let moved = renderer.follow(pointer, seconds);
            // A scene that only follows the pointer draws when it moves, and otherwise looks
            // again a frame later without drawing.
            if !moved && !renderer.animated() && !paused && !force {
                let wait = std::time::Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
                scene.timer = due_after(&self.handle, key, id, wait);
                return;
            }
        }
        if let Err(message) = advance(&self.gl, fps, accent, speed, !paused, scene) {
            self.shared_failed(key, message);
            return;
        }
        // A paused monitor keeps the frame it has while the others go on, unless the canvas
        // it shows was just made anew.
        for index in users {
            if !force && self.is_paused(&self.screens[index].name) {
                continue;
            }
            if self.screens[index].frame_pending {
                self.screens[index].stale = true;
            } else {
                self.draw(index);
            }
        }
    }

    /// Where the pointer is over the scene: over the monitor it was last seen on, through the
    /// part of the scene that monitor shows.
    fn shared_pointer(&mut self, key: &SceneKey, users: &[usize]) -> [f32; 2] {
        for &index in users {
            self.pointer(index);
        }
        let index = users
            .iter()
            .copied()
            .find(|&index| Some(self.screens[index].id) == self.pointed_last)
            .unwrap_or(users[0]);
        let screen = &self.screens[index];
        let (Some(shown), Some(target), Some(shared)) = (
            screen.shown.as_ref(),
            screen.target.as_ref(),
            self.scenes.get(key),
        ) else {
            return screen.pointer;
        };
        let mut uv = picture::placement(shown.show.fit, shared.scene.canvas.size, target.size());
        if shown.show.look.unwrap_or_default().flip {
            uv = picture::flipped(uv);
        }
        super::pointer::through(uv, screen.pointer)
    }

    /// Every monitor showing a scene that stopped drawing gives it up.
    fn shared_failed(&mut self, key: &SceneKey, message: String) {
        let users = self.scene_users(key);
        for index in users {
            let Some(shown) = self.screens[index].shown.take() else {
                continue;
            };
            if let Some(fade) = self.screens[index].fade.take() {
                self.gl.delete(fade.before.texture);
            }
            let output = self.screens[index].name.clone();
            self.give_up(index, shown.show.clone());
            protocol::emit(&Event::Error {
                output: Some(output),
                source: Some(shown.show.source),
                message: message.clone(),
            });
        }
        if let Some(shared) = self.scenes.remove(key) {
            self.drop_scene(shared.scene);
        }
        protocol::emit(&self.status());
    }

    /// A paused scene keeps its clock still, so it resumes where it stopped.
    pub(super) fn pause_shared(&mut self) {
        let keys: Vec<SceneKey> = self.scenes.keys().cloned().collect();
        for key in keys {
            let users = self.scene_users(&key);
            if users.is_empty() {
                continue;
            }
            let paused = users
                .iter()
                .all(|&index| self.is_paused(&self.screens[index].name));
            let waiting = users.iter().all(|&index| self.screens[index].frame_pending);
            let Some(shared) = self.scenes.get_mut(&key) else {
                continue;
            };
            if paused {
                shared.scene.clock.pause();
                if let Some(token) = shared.scene.timer.take() {
                    self.handle.remove(token);
                }
            } else if !waiting {
                self.step_shared(&key);
            }
        }
    }
}
