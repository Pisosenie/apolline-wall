use std::path::PathBuf;
use std::sync::mpsc;

use super::App;
use crate::picture::{self, Pixels};
use crate::protocol::{self, Event};
use smithay_client_toolkit::reexports::calloop::channel;

pub(super) struct Job {
    pub output: String,
    pub source: PathBuf,
    pub generation: u64,
    pub pixels: Pixels,
}

pub(super) struct Saved {
    output: String,
    source: PathBuf,
    generation: u64,
    result: Result<PathBuf, String>,
}

pub(super) fn run(jobs: mpsc::Receiver<Job>, done: channel::Sender<Saved>) {
    for job in jobs {
        let path = apolline_wall_protocol::source::poster_path(&job.source);
        let result = picture::save_png(
            &path,
            &job.pixels.rgba,
            (job.pixels.width, job.pixels.height),
        )
        .map(|()| path);
        let Job {
            output,
            source,
            generation,
            pixels,
        } = job;
        drop(pixels);
        if done
            .send(Saved {
                output,
                source,
                generation,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

impl App {
    pub(super) fn queue_poster(
        &mut self,
        output: String,
        source: PathBuf,
        generation: u64,
        pixels: Result<Pixels, String>,
    ) {
        match pixels {
            Ok(pixels) => {
                self.pending_posters.insert(
                    output.clone(),
                    Job {
                        output,
                        source,
                        generation,
                        pixels,
                    },
                );
                self.pump_posters();
            }
            Err(message) => self.posted(Saved {
                output,
                source,
                generation,
                result: Err(message),
            }),
        }
    }

    pub(super) fn pump_posters(&mut self) {
        let keys: Vec<_> = self.pending_posters.keys().cloned().collect();
        for key in keys {
            let Some(job) = self.pending_posters.remove(&key) else {
                continue;
            };
            match self.posters.try_send(job) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(job)) => {
                    self.pending_posters.insert(key, job);
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(job)) => self.posted(Saved {
                    output: job.output,
                    source: job.source,
                    generation: job.generation,
                    result: Err("preview writer stopped".into()),
                }),
            }
        }
    }

    pub(super) fn posted(&self, saved: Saved) {
        if !self.screens.iter().any(|screen| {
            screen.name == saved.output
                && screen
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.generation == saved.generation)
        }) {
            return;
        }
        let (ready, warning) = completion(saved.output, saved.source, saved.result);
        protocol::emit(&ready);
        if let Some(warning) = warning {
            protocol::emit(&warning);
        }
    }
}

fn completion(
    output: String,
    source: PathBuf,
    result: Result<PathBuf, String>,
) -> (Event, Option<Event>) {
    let (poster, warning) = match result {
        Ok(poster) => (poster, None),
        Err(message) => (
            source.clone(),
            Some(Event::Error {
                output: Some(output.clone()),
                source: None,
                message: format!("Wallpaper is running; cannot save its preview: {message}"),
            }),
        ),
    };
    (
        Event::Ready {
            output,
            source,
            poster,
        },
        warning,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_errors_do_not_report_a_running_wallpaper_as_failed() {
        let source = PathBuf::from("/synthetic/scene");
        let (ready, warning) =
            completion("DP-1".into(), source.clone(), Err("read-only cache".into()));
        assert!(
            matches!(ready, Event::Ready { output, source: shown, poster } if output == "DP-1" && shown == source && poster == source)
        );
        assert!(matches!(
            warning,
            Some(Event::Error {
                output: Some(_),
                source: None,
                ..
            })
        ));
        let (ready, warning) =
            completion("DP-1".into(), source, Ok("/synthetic/poster.png".into()));
        assert!(matches!(ready, Event::Ready { .. }));
        assert!(warning.is_none());
    }
}
