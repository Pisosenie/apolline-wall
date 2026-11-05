//! A wallpaper that takes the daemon down with it, twice in a row, is skipped until it's set
//! again. While one is loading and for a few seconds after it shows, its path waits in a small
//! file next to the state; finding it there at the next start means the daemon died with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};

use super::App;
use crate::protocol::{self, Event};
use crate::state;

/// How long a wallpaper has to keep showing before it counts as safe.
const SAFE: Duration = Duration::from_secs(10);
/// Deaths in a row that get a wallpaper skipped.
const TIMES: u32 = 2;

/// What showed in the last few seconds, by when; what is loading comes from the screens.
#[derive(Default)]
pub(super) struct Trying {
    shown: HashMap<PathBuf, Instant>,
}

impl App {
    fn trying_file(&self) -> Option<PathBuf> {
        self.persist
            .as_ref()
            .map(|persist| persist.path.with_file_name("trying"))
    }

    /// Writes down what is loading now and what showed in the last few seconds, which is what
    /// a crash right now would be blamed on.
    pub(super) fn note_trying(&mut self) {
        if self.persist.is_none() {
            return;
        }
        self.trying.shown.retain(|_, at| at.elapsed() < SAFE);
        let mut suspects: Vec<PathBuf> = self
            .screens
            .iter()
            .filter_map(|screen| screen.loading.as_ref())
            .map(|loading| loading.show.source.clone())
            .chain(self.trying.shown.keys().cloned())
            .collect();
        suspects.sort();
        suspects.dedup();
        self.write_trying(&suspects);
    }

    /// A wallpaper just showed; if it is still showing a few seconds on, it's safe.
    pub(super) fn showing(&mut self, source: PathBuf) {
        if self.persist.is_none() {
            return;
        }
        self.trying.shown.insert(source.clone(), Instant::now());
        self.note_trying();
        let timer = Timer::from_duration(SAFE);
        let _ = self
            .handle
            .insert_source(timer, move |_, _, app: &mut App| {
                app.note_trying();
                let still = app.screens.iter().any(|screen| {
                    screen
                        .shown
                        .as_ref()
                        .is_some_and(|shown| shown.show.source == source)
                });
                if still {
                    app.forgive(&source);
                }
                TimeoutAction::Drop
            });
    }

    /// Setting a wallpaper on purpose gives it another chance.
    pub(super) fn forgive(&mut self, source: &Path) {
        let Some(persist) = &mut self.persist else {
            return;
        };
        if persist
            .state
            .crashes
            .remove(&source.to_string_lossy().into_owned())
            .is_some()
            && let Err(message) = state::save(&persist.path, &persist.state)
        {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
    }

    fn write_trying(&self, suspects: &[PathBuf]) {
        let Some(path) = self.trying_file() else {
            return;
        };
        if suspects.is_empty() {
            let _ = std::fs::remove_file(&path);
            return;
        }
        let lines: Vec<String> = suspects
            .iter()
            .map(|source| source.to_string_lossy().into_owned())
            .collect();
        let fresh = path.with_extension("new");
        if std::fs::write(&fresh, lines.join("\n")).is_ok() {
            let _ = std::fs::rename(&fresh, &path);
        }
    }

    /// At start: counts what the last run died with, and leaves out what died twice.
    pub(super) fn recover(&mut self) {
        let Some(path) = self.trying_file() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let Some(persist) = &mut self.persist else {
            return;
        };
        let mut skipped = Vec::new();
        for source in text.lines().filter(|line| !line.trim().is_empty()).take(64) {
            let times = persist.state.crashes.entry(source.to_owned()).or_insert(0);
            *times += 1;
            if *times >= TIMES {
                skipped.push(PathBuf::from(source));
            }
        }
        for source in &skipped {
            persist.state.forget(source);
            persist.good.forget(source);
        }
        if let Err(message) = state::save(&persist.path, &persist.state) {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
        for source in skipped {
            let wanted = |show: &crate::protocol::Show| show.source == source;
            if self.selection.everywhere.as_ref().is_some_and(wanted) {
                self.selection.everywhere = None;
            }
            self.selection.per_output.retain(|_, show| !wanted(show));
            protocol::emit(&Event::Error {
                output: None,
                source: Some(source.clone()),
                message: format!(
                    "{} stopped apolline-wall twice in a row, so it isn't shown until you set it again",
                    source.display()
                ),
            });
        }
    }

    /// A normal exit isn't a crash.
    pub(super) fn done_trying(&mut self) {
        self.write_trying(&[]);
    }
}
