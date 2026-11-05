//! Still images are decoded once per source and fitted once per requested size. Replaced
//! requests stop at the next boundary, before more pixels are allocated or sent to the event loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use crate::picture::{self, Pixels, Source};
use crate::protocol::Fit;

/// Kept by the screen while it wants a load; dropping it cancels that load.
#[derive(Default)]
pub struct Pending(Arc<AtomicBool>);

impl Pending {
    pub fn ticket(&self) -> Ticket {
        Ticket(Arc::clone(&self.0))
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub struct Ticket(Arc<AtomicBool>);

impl Ticket {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct Batch {
    pub source: PathBuf,
    pub jobs: Vec<Job>,
}

pub struct Job {
    pub screen: u64,
    pub generation: u64,
    pub size: (u32, u32),
    pub fit: Fit,
    pub properties: apolline_scene::properties::Values,
    pub ticket: Ticket,
}

pub struct Loaded {
    pub screen: u64,
    pub generation: u64,
    pub source: PathBuf,
    pub result: Result<Content, String>,
}

pub enum Content {
    Image(Arc<Pixels>),
    Scene(Arc<apolline_scene::Prepared>),
    External(String),
}

pub fn run(batches: mpsc::Receiver<Batch>, mut send: impl FnMut(Loaded) -> bool) {
    for batch in batches {
        let connected = if batch.source.is_dir() {
            scene(batch, &mut send)
        } else {
            process(
                batch,
                picture::decode,
                |source, (w, h), fit| picture::fit(source, w, h, fit),
                &mut send,
            )
        };
        #[cfg(target_env = "gnu")]
        // The worker sleeps after a batch; return its large temporary allocations now.
        unsafe {
            libc::malloc_trim(0)
        };
        if !connected {
            break;
        }
    }
}

fn scene(mut batch: Batch, mut send: impl FnMut(Loaded) -> bool) -> bool {
    batch.jobs.retain(|job| !job.ticket.cancelled());
    if batch.jobs.is_empty() {
        return true;
    }
    let mut prepared = None;
    for job in batch.jobs {
        if !job.ticket.cancelled() {
            let key = serde_json::to_string(&job.properties).unwrap_or_default();
            if prepared.as_ref().is_none_or(|(cached, _)| cached != &key) {
                let result = if std::env::var_os("APOLLINE_WALL_SCENE_RENDERER").is_some() {
                    Err("an external scene renderer was selected".into())
                } else {
                    crate::native::load(&batch.source, &job.properties).map(Arc::new)
                };
                prepared = Some((key, result));
            }
            let Some((_, scene)) = &prepared else {
                continue;
            };
            let content = match scene {
                Ok(scene) => Ok(Content::Scene(Arc::clone(scene))),
                Err(reason) if job.properties.is_empty() => Ok(Content::External(reason.clone())),
                Err(reason) => Err(format!(
                    "user property edits require a supported native scene: {reason}"
                )),
            };
            if !send(Loaded {
                screen: job.screen,
                generation: job.generation,
                source: batch.source.clone(),
                result: content,
            }) {
                return false;
            }
        }
    }
    true
}

fn process(
    mut batch: Batch,
    decode: impl FnOnce(&Path) -> Result<Source, String>,
    mut fit: impl FnMut(&Source, (u32, u32), Fit) -> Result<Pixels, String>,
    mut send: impl FnMut(Loaded) -> bool,
) -> bool {
    batch.jobs.retain(|job| !job.ticket.cancelled());
    if batch.jobs.is_empty() {
        return true;
    }
    let decoded = decode(&batch.source);
    let mut fitted = HashMap::new();
    for job in batch.jobs {
        if job.ticket.cancelled() {
            continue;
        }
        let result = fitted.entry((job.size, job.fit)).or_insert_with(|| {
            decoded
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|source| fit(source, job.size, job.fit))
                .map(Arc::new)
        });
        if !job.ticket.cancelled()
            && !send(Loaded {
                screen: job.screen,
                generation: job.generation,
                source: batch.source.clone(),
                result: result.clone().map(Content::Image),
            })
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(jobs: &[(u64, (u32, u32))]) -> (Batch, Vec<Pending>) {
        let pending: Vec<_> = jobs.iter().map(|_| Pending::default()).collect();
        let batch = Batch {
            source: PathBuf::from("test.png"),
            jobs: jobs
                .iter()
                .zip(&pending)
                .map(|(&(screen, size), load)| Job {
                    screen,
                    generation: screen,
                    size,
                    fit: Fit::Cover,
                    properties: Default::default(),
                    ticket: load.ticket(),
                })
                .collect(),
        };
        (batch, pending)
    }

    fn source() -> Source {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "wall-loading-{}-{}.png",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        image::RgbaImage::new(4, 4).save(&path).unwrap();
        let source = picture::decode(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        source
    }

    #[test]
    fn obsolete_requests_never_decode() {
        let (batch, pending) = batch(&[(1, (2, 2))]);
        drop(pending);
        assert!(process(
            batch,
            |_| panic!("decoded a cancelled request"),
            |_, _, _| panic!("resized a cancelled request"),
            |_| panic!("sent a cancelled request")
        ));
    }

    #[test]
    fn identical_monitors_share_a_single_resize_and_pixel_buffer() {
        let (batch, _pending) = batch(&[(1, (2, 2)), (2, (2, 2)), (3, (3, 2))]);
        let mut sizes = Vec::new();
        let mut loaded = Vec::new();
        assert!(process(
            batch,
            |_| Ok(source()),
            |source, (w, h), fit| {
                sizes.push((w, h));
                picture::fit(source, w, h, fit)
            },
            |result| {
                loaded.push(result);
                true
            }
        ));
        assert_eq!(sizes, [(2, 2), (3, 2)]);
        assert_eq!(
            loaded
                .iter()
                .map(|r| (r.screen, r.generation))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 2), (3, 3)]
        );
        let pixels = |at: usize| match loaded[at].result.as_ref().unwrap() {
            Content::Image(pixels) => pixels,
            _ => panic!("expected an image"),
        };
        assert!(Arc::ptr_eq(pixels(0), pixels(1)));
        assert!(!Arc::ptr_eq(pixels(0), pixels(2)));
    }

    #[test]
    fn cancellation_during_decode_does_not_delay_another_monitor() {
        let (batch, mut pending) = batch(&[(1, (2, 2)), (2, (3, 2))]);
        let mut screens = Vec::new();
        process(
            batch,
            |_| {
                drop(pending.remove(0));
                Ok(source())
            },
            |source, (w, h), fit| {
                assert_eq!((w, h), (3, 2));
                picture::fit(source, w, h, fit)
            },
            |result| {
                screens.push(result.screen);
                true
            },
        );
        assert_eq!(screens, [2]);
    }

    #[test]
    fn cancellation_during_resize_discards_its_result() {
        let (batch, mut pending) = batch(&[(1, (2, 2))]);
        process(
            batch,
            |_| Ok(source()),
            |source, (w, h), fit| {
                pending.clear();
                picture::fit(source, w, h, fit)
            },
            |_| panic!("sent a cancelled result"),
        );
    }

    #[test]
    fn errors_reach_only_the_screens_that_still_want_the_file() {
        let (batch, mut pending) = batch(&[(1, (2, 2)), (2, (3, 2))]);
        drop(pending.remove(0));
        let mut screens = Vec::new();
        process(
            batch,
            |_| Err("broken image".into()),
            |_, _, _| panic!("resized a broken image"),
            |result| {
                assert_eq!(result.result.err().unwrap(), "broken image");
                screens.push(result.screen);
                true
            },
        );
        assert_eq!(screens, [2]);
    }

    #[test]
    fn a_closed_receiver_stops_the_worker() {
        let (batch, _pending) = batch(&[(1, (2, 2)), (2, (3, 2))]);
        let mut resized = 0;
        assert!(!process(
            batch,
            |_| Ok(source()),
            |source, (w, h), fit| {
                resized += 1;
                picture::fit(source, w, h, fit)
            },
            |_| false
        ));
        assert_eq!(resized, 1);
    }
}
