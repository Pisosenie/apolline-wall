use std::path::{Path, PathBuf};

use smithay_client_toolkit::reexports::calloop::channel;

use super::{App, Picture, Video};
use crate::picture;
use crate::protocol::{self, Event, Fit};
use crate::video::{Happened, Mpv, Player};
use apolline_wall_protocol::source::{poster_path, project_video};

impl App {
    pub(super) fn start_video(&mut self, index: usize) {
        let Some(source) = self.screens[index]
            .loading
            .as_ref()
            .map(|loading| loading.show.source.clone())
        else {
            return;
        };
        if !self.videos.contains_key(&source) {
            match self.open_video(&source) {
                Ok(video) => {
                    self.videos.insert(source.clone(), video);
                }
                Err(message) => {
                    if let Some(loading) = self.screens[index].loading.take() {
                        self.give_up(index, loading.show);
                    }
                    protocol::emit(&Event::Error {
                        output: Some(self.screens[index].name.clone()),
                        source: Some(source),
                        message,
                    });
                    return;
                }
            }
        }
        if self.videos.get(&source).is_some_and(|video| video.started) {
            self.attach(index);
        }
        self.sync_pauses();
    }

    fn open_video(&mut self, source: &Path) -> Result<Video, String> {
        // A Workshop video project plays the file it names.
        let file = if source.is_dir() {
            project_video(source)
                .ok_or_else(|| format!("{}: no video in the project", source.display()))?
        } else {
            source.to_path_buf()
        };
        if !file.is_file() {
            return Err(format!("{}: no such file", source.display()));
        }
        let mpv = match &self.mpv {
            Some(mpv) => mpv.clone(),
            None => self.mpv.insert(Mpv::open()?).clone(),
        };
        self.gl.current()?;
        Ok(Video {
            player: Player::new(&mpv, &file, self.gl.display(), self.platform, &self.wake)?,
            canvas: None,
            size: None,
            started: false,
            waiting: false,
            paused: false,
            poster: None,
            unannounced: Default::default(),
        })
    }

    fn attach(&mut self, index: usize) {
        let Some(loading) = self.screens[index].loading.take() else {
            return;
        };
        let source = loading.show.source.clone();
        let output = self.screens[index].name.clone();
        let generation = loading.generation;
        self.present(index, Picture::Video(source.clone()), loading);
        self.restyle(index);
        let Some(video) = self.videos.get_mut(&source) else {
            return;
        };
        match &video.poster {
            Some(poster) => protocol::emit(&Event::Ready {
                output,
                source,
                poster: poster.clone(),
            }),
            None => video.unannounced.wait(output, generation),
        }
    }

    pub(super) fn tick(&mut self) {
        let sources: Vec<PathBuf> = self.videos.keys().cloned().collect();
        for source in sources {
            let Some(video) = self.videos.get(&source) else {
                continue;
            };
            let happenings = video.player.happenings();
            let fresh = video.player.has_frame();
            for happened in happenings {
                self.happened(&source, happened);
            }
            if let Some(video) = self.videos.get_mut(&source) {
                video.waiting |= fresh;
                self.pump(&source);
            }
        }
    }

    fn happened(&mut self, source: &Path, happened: Happened) {
        match happened {
            Happened::Size(w, h) => {
                if let Some(video) = self.videos.get_mut(source) {
                    video.size = Some((w, h));
                }
                self.fit_canvas(source);
            }
            Happened::Saved(result) => {
                let poster = match result {
                    Ok(()) => poster_path(source),
                    // Only a warning: the video plays on. Without its source the error doesn't
                    // read as the wallpaper failing, and the poster stays the video itself
                    // (a project folder's video file, not the folder), which a host can tell
                    // from a picture.
                    Err(message) => {
                        protocol::emit(&Event::Error {
                            output: None,
                            source: None,
                            message: format!("{}: poster frame: {message}", source.display()),
                        });
                        project_video(source).unwrap_or_else(|| source.to_path_buf())
                    }
                };
                let Some(video) = self.videos.get_mut(source) else {
                    return;
                };
                video.poster = Some(poster.clone());
                let mut pending = std::mem::take(&mut video.unannounced);
                let recipients = pending.take(|output, generation| {
                    self.screens.iter().find(|screen| screen.name == output)
                        .and_then(|screen| screen.shown.as_ref())
                        .is_some_and(|shown| shown.generation == generation
                            && matches!(&shown.picture, Picture::Video(current) if current == source))
                });
                for output in recipients {
                    protocol::emit(&Event::Ready {
                        output,
                        source: source.to_path_buf(),
                        poster: poster.clone(),
                    });
                }
            }
            Happened::Failed(message) => self.fail(source, message),
        }
    }

    /// Renders the next frame only when some monitor showing the video can take it, so a
    /// wallpaper the compositor stops asking for stops decoding too.
    pub(super) fn pump(&mut self, source: &Path) {
        let users = self.users(source);
        let Some(video) = self.videos.get(source) else {
            return;
        };
        let Some(canvas) = &video.canvas else {
            return;
        };
        let ready = !video.started || users.iter().any(|&i| !self.screens[i].frame_pending);
        if !video.waiting || !ready {
            return;
        }
        let rendered = self
            .gl
            .current()
            .and_then(|()| video.player.render(canvas.framebuffer(), canvas.size));
        if let Err(message) = rendered {
            self.fail(source, message);
            return;
        }
        let Some(video) = self.videos.get_mut(source) else {
            return;
        };
        video.waiting = false;
        let first = !std::mem::replace(&mut video.started, true);
        if first {
            // Read back from our own canvas: mpv's screenshot-to-file fails under its render
            // API on some drivers (NVIDIA's, for one).
            let read = video
                .canvas
                .as_ref()
                .filter(|canvas| canvas.size.0 > 0 && canvas.size.1 > 0)
                .ok_or_else(|| "the video has no canvas".to_owned())
                .and_then(|canvas| Ok((self.gl.read(canvas)?, canvas.size)));
            match read {
                Ok((rgba, size)) => self.save_poster(source, rgba, size),
                Err(message) => self.happened(source, Happened::Saved(Err(message))),
            }
        }
        for index in users {
            let screen = &mut self.screens[index];
            let arriving = screen
                .loading
                .as_ref()
                .is_some_and(|loading| loading.show.source == source);
            if arriving {
                if first {
                    self.attach(index);
                }
            } else if screen.frame_pending {
                screen.stale = true;
            } else {
                self.draw(index);
            }
        }
    }

    /// Encodes the first frame on a thread of its own: a 4K PNG takes long enough to hold up
    /// every monitor and the socket.
    fn save_poster(&mut self, source: &Path, rgba: Vec<u8>, size: (u32, u32)) {
        let (sender, saved) = channel::channel();
        let video = source.to_path_buf();
        let token = std::rc::Rc::new(std::cell::Cell::new(None));
        let own = token.clone();
        let watched = self
            .handle
            .insert_source(saved, move |event, _, app: &mut App| {
                if let channel::Event::Msg(result) = event {
                    app.happened(&video, Happened::Saved(result));
                }
                // One message, then the channel goes.
                if let Some(token) = own.take() {
                    app.handle.remove(token);
                }
            });
        match watched {
            Ok(watched) => token.set(Some(watched)),
            Err(err) => {
                self.happened(source, Happened::Saved(Err(err.to_string())));
                return;
            }
        }
        let path = poster_path(source);
        std::thread::spawn(move || {
            // mpv draws into the canvas top row first, the other way from GL.
            let row = size.0 as usize * 4;
            let rgba: Vec<u8> = rgba.chunks_exact(row).rev().flatten().copied().collect();
            let _ = sender.send(crate::picture::save_png(&path, &rgba, size));
        });
    }

    pub(super) fn fit_canvases(&mut self) {
        let sources: Vec<PathBuf> = self.videos.keys().cloned().collect();
        for source in sources {
            self.fit_canvas(&source);
        }
    }

    /// The canvas is the video at the size the most demanding monitor needs, never upscaled.
    fn fit_canvas(&mut self, source: &Path) {
        let targets: Vec<(Fit, (u32, u32))> = self
            .users(source)
            .into_iter()
            .filter_map(|index| {
                let screen = &self.screens[index];
                let show = match &screen.loading {
                    Some(loading) if loading.show.source == source => &loading.show,
                    _ => &screen.shown.as_ref()?.show,
                };
                Some((show.fit, screen.target.as_ref()?.size()))
            })
            .collect();
        let Some(video) = self.videos.get(source) else {
            return;
        };
        let Some(size) = video.size else {
            return;
        };
        let wanted = picture::canvas_size(size, targets);
        if video
            .canvas
            .as_ref()
            .is_some_and(|canvas| canvas.size == wanted)
        {
            return;
        }
        let canvas = match self.gl.canvas(wanted) {
            Ok(canvas) => canvas,
            Err(message) => {
                self.fail(source, message);
                return;
            }
        };
        let Some(video) = self.videos.get_mut(source) else {
            return;
        };
        if let Some(old) = video.canvas.replace(canvas) {
            self.gl.free(old);
        }
        video.waiting |= video.started;
        self.pump(source);
    }

    pub(super) fn sync_pauses(&mut self) {
        let sources: Vec<PathBuf> = self.videos.keys().cloned().collect();
        for source in sources {
            let users = self.users(&source);
            let paused = !users.is_empty()
                && users
                    .iter()
                    .all(|&index| self.is_paused(&self.screens[index].name));
            if let Some(video) = self.videos.get_mut(&source)
                && video.paused != paused
            {
                video.player.set_paused(paused);
                video.paused = paused;
            }
        }
    }

    fn fail(&mut self, source: &Path, message: String) {
        for index in 0..self.screens.len() {
            let screen = &self.screens[index];
            let loading = screen
                .loading
                .as_ref()
                .is_some_and(|loading| loading.show.source == source);
            let showing = matches!(
                screen.shown.as_ref().map(|shown| &shown.picture),
                Some(Picture::Video(shown)) if shown == source
            );
            if loading || showing {
                protocol::emit(&Event::Error {
                    output: Some(screen.name.clone()),
                    source: Some(source.to_path_buf()),
                    message: message.clone(),
                });
            }
            if loading && let Some(loading) = self.screens[index].loading.take() {
                self.give_up(index, loading.show);
            } else if showing && let Some(shown) = &self.screens[index].shown {
                self.give_up(index, shown.show.clone());
            }
            if showing {
                self.screens[index].shown = None;
                if let Some(fade) = self.screens[index].fade.take() {
                    self.gl.delete(fade.before.texture);
                }
            }
        }
        self.close(source);
        protocol::emit(&self.status());
    }

    pub(super) fn close_unused_videos(&mut self) {
        let unused: Vec<PathBuf> = self
            .videos
            .keys()
            .filter(|source| self.users(source).is_empty())
            .cloned()
            .collect();
        for source in unused {
            self.close(&source);
        }
    }

    pub(super) fn close_videos(&mut self) {
        let sources: Vec<PathBuf> = self.videos.keys().cloned().collect();
        for source in sources {
            self.close(&source);
        }
    }

    /// libmpv frees its GL objects on the way out, so the context has to be current.
    fn close(&mut self, source: &Path) {
        let _ = self.gl.current();
        if let Some(Video { player, canvas, .. }) = self.videos.remove(source) {
            drop(player);
            if let Some(canvas) = canvas {
                self.gl.free(canvas);
            }
        }
    }

    fn users(&self, source: &Path) -> Vec<usize> {
        self.screens
            .iter()
            .enumerate()
            .filter(|(_, screen)| {
                let loading = screen
                    .loading
                    .as_ref()
                    .is_some_and(|loading| loading.show.source == source);
                let showing = matches!(
                    screen.shown.as_ref().map(|shown| &shown.picture),
                    Some(Picture::Video(shown)) if shown == source
                );
                loading || showing
            })
            .map(|(index, _)| index)
            .collect()
    }
}
