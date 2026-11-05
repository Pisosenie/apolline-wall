use std::time::{Duration, Instant};

use super::{App, Fade, Loading, Picture, Shown, give_back_memory};
use super::{native::Native, scenery};
use crate::gl;
use crate::loading::{Content, Loaded};
use crate::picture::{self, UPSIDE_DOWN, WHOLE};
use crate::protocol::{self, Event};

impl App {
    pub(super) fn loaded(&mut self, loaded: Loaded) {
        let Some(index) = self.index_of(loaded.screen) else {
            return;
        };
        let screen = &mut self.screens[index];
        if screen.loading.as_ref().map(|l| l.generation) != Some(loaded.generation) {
            return;
        }
        let Some(loading) = screen.loading.take() else {
            return;
        };
        let texture = match loaded.result {
            Ok(Content::Image(pixels)) => self
                .gl
                .upload(&pixels)
                .map(|texture| (Picture::Still(texture), loading.show.source.clone())),
            Ok(Content::Scene(scene)) => {
                let native = self.native_scene(index, &scene, &loading);
                drop(scene);
                give_back_memory();
                match native {
                    Ok(native) => {
                        let (picture, pixels) = match native {
                            Native::Still(texture, pixels) => (Picture::Native(texture), pixels),
                            Native::Moving(scene, pixels) => {
                                (Picture::Shader(Box::new(scene)), pixels)
                            }
                        };
                        let generation = loading.generation;
                        let source = loading.show.source.clone();
                        let output = self.screens[index].name.clone();
                        self.present(index, picture, loading);
                        self.queue_poster(output, source, generation, Ok(pixels));
                        protocol::emit(&self.status());
                        return;
                    }
                    Err(reason) => {
                        self.screens[index].loading = Some(loading);
                        self.start_external(index, reason);
                        return;
                    }
                }
            }
            Ok(Content::External(reason)) => {
                self.screens[index].loading = Some(loading);
                self.start_external(index, reason);
                return;
            }
            Err(message) => Err(message),
        };
        give_back_memory();
        let output = self.screens[index].name.clone();
        match texture {
            Ok((picture, poster)) => {
                let source = loading.show.source.clone();
                self.present(index, picture, loading);
                protocol::emit(&Event::Ready {
                    output,
                    poster,
                    source,
                });
                protocol::emit(&self.status());
            }
            Err(message) => {
                self.give_up(index, loading.show);
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(loaded.source),
                    message,
                });
            }
        }
    }

    pub(super) fn present(&mut self, index: usize, picture: Picture, loading: Loading) {
        if let Some(persist) = &mut self.persist {
            persist.good.confirm(&loading.show, &persist.state);
        }
        let paused = self.is_paused(&self.screens[index].name);
        let was_external = matches!(
            self.screens[index]
                .shown
                .as_ref()
                .map(|shown| &shown.picture),
            Some(Picture::External(_))
        );
        let external = matches!(picture, Picture::External(_));
        let before = self.screens[index]
            .shown
            .take()
            .and_then(|old| self.retire(index, old));
        let screen = &mut self.screens[index];
        if let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before.texture);
        }
        let duration = Duration::from_millis(loading.show.transition_ms.into());
        match before {
            Some(before) if !paused && !external && !duration.is_zero() => {
                screen.fade = Some(Fade {
                    before,
                    start: Instant::now(),
                    duration,
                })
            }
            Some(before) => self.gl.delete(before.texture),
            None => {}
        }
        self.pending_posters.remove(&screen.name);
        screen.shown = Some(Shown {
            generation: loading.generation,
            picture,
            show: loading.show,
            size: loading.size,
        });
        if was_external && !external {
            self.unhide(index);
        }
        self.draw(index);
        self.close_unused_videos();
        self.sync_pauses();
    }

    fn retire(&self, index: usize, old: Shown) -> Option<gl::Layer> {
        match old.picture {
            Picture::Still(texture) => Some(gl::Layer::new(texture, WHOLE, old.show.look)),
            Picture::Native(texture) => Some(gl::Layer::new(texture, UPSIDE_DOWN, old.show.look)),
            Picture::Video(source) => {
                let canvas = self.videos.get(&source)?.canvas.as_ref()?;
                let target = self.screens[index].target.as_ref()?;
                Some(gl::Layer::new(
                    self.gl.snapshot(canvas).ok()?,
                    picture::placement(old.show.fit, canvas.size, target.size()),
                    old.show.look,
                ))
            }
            Picture::Shader(scene) => {
                let texture = self.gl.snapshot(&scene.canvas).ok();
                self.drop_scene(*scene);
                Some(gl::Layer::new(texture?, UPSIDE_DOWN, old.show.look))
            }
            Picture::External(external) => {
                scenery::stop(&external);
                None
            }
        }
    }

    pub(super) fn draw(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let screen = &self.screens[index];
        let (Some(target), Some(shown)) = (&screen.target, &screen.shown) else {
            return;
        };
        let after = match &shown.picture {
            Picture::Still(texture) => gl::Layer::new(*texture, WHOLE, shown.show.look),
            Picture::Native(texture) => gl::Layer::new(*texture, UPSIDE_DOWN, shown.show.look),
            Picture::Video(source) => {
                let Some(canvas) = self.videos.get(source).and_then(|v| v.canvas.as_ref()) else {
                    return;
                };
                gl::Layer::new(
                    canvas.texture,
                    picture::placement(shown.show.fit, canvas.size, target.size()),
                    shown.show.look,
                )
            }
            Picture::Shader(scene) => {
                gl::Layer::new(scene.canvas.texture, UPSIDE_DOWN, shown.show.look)
            }
            Picture::External(_) => return,
        };
        let progress = match &screen.fade {
            Some(fade) if !paused => {
                fade.start.elapsed().as_secs_f32() / fade.duration.as_secs_f32()
            }
            _ => 1.0,
        };
        let fading = progress < 1.0;
        let playing = matches!(shown.picture, Picture::Video(_) | Picture::Shader(_)) && !paused;
        let before = screen
            .fade
            .as_ref()
            .filter(|_| fading)
            .map(|fade| fade.before);
        if (fading || playing) && !screen.frame_pending {
            self.want_frame(index);
        }
        let screen = &mut self.screens[index];
        screen.stale = false;
        let Some(target) = &screen.target else {
            return;
        };
        let eased = progress * progress * (3.0 - 2.0 * progress);
        if let Err(message) = self.gl.draw(target, before, after, eased.min(1.0)) {
            protocol::emit(&Event::Error {
                output: Some(screen.name.clone()),
                source: None,
                message,
            });
        }
        if !fading && let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before.texture);
        }
    }

    pub(super) fn frame_done(&mut self, index: usize) {
        self.screens[index].frame_pending = false;
        if let Some(Picture::Video(source)) = self.screens[index].shown.as_ref().map(|s| &s.picture)
        {
            let source = source.clone();
            if self.videos.get(&source).is_some_and(|video| video.waiting) {
                self.pump(&source);
            }
        }
        let shader = matches!(
            self.screens[index].shown.as_ref().map(|s| &s.picture),
            Some(Picture::Shader(_))
        );
        if shader && !self.is_paused(&self.screens[index].name) {
            self.tick_scene(index);
        }
        let screen = &self.screens[index];
        if !screen.frame_pending && (screen.fade.is_some() || screen.stale) {
            self.draw(index);
        }
    }
}
