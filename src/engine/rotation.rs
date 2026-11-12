use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};

use apolline_wall_protocol::playlist::{folder_sources, order};

use super::App;
use crate::protocol::{self, Event, Playlist, Show, Step};

pub(super) struct Rotation {
    list: Playlist,
    index: usize,
    deadline: Instant,
    suspended: bool,
    failed: HashSet<PathBuf>,
    /// The folder couldn't be read at the last look.
    unreadable: bool,
}

impl Rotation {
    /// Its monitor now on another connector.
    pub(super) fn moved_to(&mut self, connector: &str) {
        self.list.output = Some(connector.to_owned());
    }

    fn show(&self, output: Option<String>) -> Show {
        let source = self.list.sources[self.index].clone();
        let settings = self.list.settings.get(&source).cloned().unwrap_or_default();
        Show {
            output,
            source,
            properties: settings.properties,
            look: settings.look,
            fit: self.list.fit,
            transition_ms: 600,
        }
    }

    fn available(&self) -> usize {
        self.list
            .sources
            .iter()
            .filter(|source| !self.failed.contains(*source))
            .count()
    }

    /// A folder always might: what was added to it joins at the next step.
    fn needs_step(&self) -> bool {
        let available = self.available();
        available > 1
            || (available == 1
                && (self.list.folder.is_some()
                    || self.failed.contains(&self.list.sources[self.index])))
    }

    fn fail(&mut self, source: &Path) -> bool {
        if self.list.sources[self.index] != source || !self.failed.insert(source.to_path_buf()) {
            return false;
        }
        self.deadline = Instant::now();
        true
    }

    /// A folder playlist's wallpapers as the folder holds them now: the one showing keeps
    /// its place, and if it's gone the next step goes to what came after it. Says whether it
    /// went.
    fn rescan(&mut self) -> Result<bool, String> {
        let Some(folder) = &self.list.folder else {
            return Ok(false);
        };
        let found = match filled(folder) {
            Ok(found) => found,
            Err(message) => {
                self.unreadable = true;
                return Err(message);
            }
        };
        // Back after it couldn't be read: what failed then, failed for being out of reach.
        if std::mem::take(&mut self.unreadable) {
            self.failed.clear();
        }
        let current = &self.list.sources[self.index];
        let (index, gone) = match found.iter().position(|path| path == current) {
            Some(index) => (index, false),
            None => {
                let after = found.partition_point(|path| order(path) < order(current));
                ((after + found.len() - 1) % found.len(), true)
            }
        };
        self.index = index;
        self.failed.retain(|path| found.contains(path));
        self.list.sources = found;
        Ok(gone)
    }

    /// The next step, after the folder is read again: one whose wallpaper left the folder
    /// shows something else even when what's left is a single one.
    fn step(&mut self, previous: bool) -> Result<bool, String> {
        let gone = self.rescan();
        // Back from a gone wallpaper is the one that came before it, where the rescan left
        // the place.
        match gone {
            Ok(true) if previous && !self.failed.contains(&self.list.sources[self.index]) => {
                Ok(true)
            }
            Ok(true) => {
                self.advance(previous);
                Ok(true)
            }
            Ok(false) => Ok(self.advance(previous)),
            Err(message) => Err(message),
        }
    }

    fn advance(&mut self, previous: bool) -> bool {
        let len = self.list.sources.len();
        for distance in 1..=len {
            let index = if previous {
                (self.index + len - distance) % len
            } else {
                (self.index + distance) % len
            };
            if !self.failed.contains(&self.list.sources[index]) {
                let changed = index != self.index;
                self.index = index;
                return changed;
            }
        }
        false
    }
}

impl App {
    pub(super) fn restore_rotation_positions(&mut self) {
        for (key, rotation) in &mut self.rotations {
            let current = key
                .as_ref()
                .and_then(|name| self.selection.per_output.get(name))
                .or(self.selection.everywhere.as_ref());
            if let Some(index) = current.and_then(|show| {
                rotation
                    .list
                    .sources
                    .iter()
                    .position(|path| path == &show.source)
            }) {
                rotation.index = index;
            }
        }
    }

    pub(super) fn rotation_status(
        &self,
        output: &str,
    ) -> Option<apolline_wall_protocol::playlist::Status> {
        let key = self.rotation_key(output)?;
        let rotation = self.rotations.get(&key)?;
        Some(apolline_wall_protocol::playlist::Status {
            position: rotation.index + 1,
            total: rotation.list.sources.len(),
            skipped: rotation.failed.len(),
            shared: key.is_none(),
            suspended: rotation.suspended,
            can_step: rotation.needs_step(),
        })
    }

    pub(super) fn rotation_key(&self, output: &str) -> Option<Option<String>> {
        let key = Some(output.to_owned());
        if self.rotations.contains_key(&key) {
            Some(key)
        } else if self.rotations.contains_key(&None) && self.selection.follows_shared(output) {
            Some(None)
        } else {
            None
        }
    }

    pub(super) fn playlist(&mut self, mut list: Playlist) {
        if let Some(folder) = &list.folder {
            match filled(folder) {
                Ok(found) => list.sources = found,
                Err(message) => {
                    return protocol::emit(&Event::Error {
                        output: list.output.clone(),
                        source: Some(folder.clone()),
                        message,
                    });
                }
            }
        }
        let mut seen = HashSet::new();
        list.sources.retain(|source| seen.insert(source.clone()));
        if list.sources.is_empty() {
            if let Some(output) = &list.output {
                if let Some(key) = self.rotation_key(output) {
                    self.rotations.remove(&key);
                }
                // Stopping a shared playlist stops its shared clock on every monitor.
                if let Some(persist) = &mut self.persist
                    && !self.rotations.contains_key(&None)
                {
                    persist.state.playlists.remove("");
                }
            } else {
                self.rotations.clear();
            }
        } else {
            if list.output.is_none() {
                self.rotations.clear();
                self.selection.per_output.clear();
                self.selection.inherited.clear();
            }
            let current = list
                .output
                .as_ref()
                .and_then(|name| self.selection.per_output.get(name))
                .or(self.selection.everywhere.as_ref())
                .map(|show| &show.source);
            let index = current
                .and_then(|source| list.sources.iter().position(|path| path == source))
                .unwrap_or(0);
            let key = list.output.clone();
            self.rotations.insert(
                key.clone(),
                Rotation {
                    deadline: Instant::now() + Duration::from_secs(list.interval_secs),
                    list,
                    index,
                    suspended: false,
                    failed: HashSet::new(),
                    unreadable: false,
                },
            );
            self.show_rotation(&key);
        }
        if let Some(persist) = &self.persist
            && let Err(message) = crate::state::save(&persist.path, &persist.state)
        {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
        self.schedule_rotations();
        protocol::emit(&self.status());
    }

    fn show_rotation(&mut self, key: &Option<String>) {
        let Some(rotation) = self.rotations.get(key) else {
            return;
        };
        let show = rotation.show(key.clone());
        if let Some(persist) = &mut self.persist {
            persist.state.keep(&show);
            if let Err(message) = crate::state::save(&persist.path, &persist.state) {
                protocol::emit(&Event::Error {
                    output: key.clone(),
                    source: None,
                    message,
                });
            }
        }
        self.selection.rotate(key, show);
        for screen in &mut self.screens {
            if key.as_ref().is_none_or(|name| name == &screen.name) {
                screen.failed = None;
            }
        }
        self.refresh();
    }

    pub(super) fn step_playlist(&mut self, step: Step) {
        let keys: Vec<_> = match step.output {
            Some(output) => self.rotation_key(&output).into_iter().collect(),
            None => self.rotations.keys().cloned().collect(),
        };
        for key in keys {
            self.advance_rotation(key, step.previous);
        }
        self.schedule_rotations();
        protocol::emit(&self.status());
    }

    fn advance_rotation(&mut self, key: Option<String>, previous: bool) {
        let Some(rotation) = self.rotations.get_mut(&key) else {
            return;
        };
        // An unreadable folder keeps playing what it held last.
        let changed = match rotation.step(previous) {
            Ok(changed) => changed,
            Err(message) => {
                protocol::emit(&Event::Error {
                    output: key.clone(),
                    source: rotation.list.folder.clone(),
                    message,
                });
                rotation.advance(previous)
            }
        };
        rotation.deadline = Instant::now() + Duration::from_secs(rotation.list.interval_secs);
        if changed {
            self.show_rotation(&key);
        }
    }

    pub(super) fn rotation_failed(&mut self, index: usize, show: &Show) {
        let name = &self.screens[index].name;
        let Some(key) = self.rotation_key(name) else {
            return;
        };
        let Some(rotation) = self.rotations.get_mut(&key) else {
            return;
        };
        if rotation.fail(&show.source) && rotation.available() == 0 {
            protocol::emit(&Event::Error {
                output: key,
                source: None,
                message: "No playable wallpapers remain in this playlist; set a wallpaper manually or start the playlist again after fixing its files".into(),
            });
        }
        self.schedule_rotations();
    }

    pub(super) fn schedule_rotations(&mut self) {
        if let Some(token) = self.rotation_timer.take() {
            self.handle.remove(token);
        }
        let now = Instant::now();
        let mut next = None;
        for (key, rotation) in &mut self.rotations {
            let screens: Vec<_> = self
                .screens
                .iter()
                .filter(|screen| match key {
                    Some(name) => name == &screen.name,
                    None => self.selection.follows_shared(&screen.name),
                })
                .collect();
            let suspended = screens.is_empty()
                || self.power.pause_reason().is_some()
                || screens.iter().any(|screen| {
                    self.manual.is_paused(&screen.name)
                        || (self.power.policy.pause_on_fullscreen && screen.covered)
                });
            if rotation.suspended && !suspended {
                rotation.deadline = if rotation
                    .failed
                    .contains(&rotation.list.sources[rotation.index])
                {
                    now
                } else {
                    now + Duration::from_secs(rotation.list.interval_secs)
                };
            }
            rotation.suspended = suspended;
            if !suspended && rotation.needs_step() {
                next =
                    Some(next.map_or(rotation.deadline, |at: Instant| at.min(rotation.deadline)));
            }
        }
        let Some(next) = next else { return };
        let timer = Timer::from_duration(next.saturating_duration_since(now));
        self.rotation_timer = self
            .handle
            .insert_source(timer, |_, _, app: &mut App| {
                app.rotation_timer = None;
                let now = Instant::now();
                let due: Vec<_> = app
                    .rotations
                    .iter()
                    .filter(|(_, rotation)| {
                        !rotation.suspended && rotation.needs_step() && rotation.deadline <= now
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                for output in due {
                    app.advance_rotation(output, false);
                }
                app.schedule_rotations();
                TimeoutAction::Drop
            })
            .ok();
    }
}

/// A folder's wallpapers, at least one.
fn filled(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let found = folder_sources(folder)?;
    if found.is_empty() {
        return Err(format!(
            "{} has no pictures, videos, shaders or Wallpaper Engine projects",
            folder.display()
        ));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rotation() -> Rotation {
        Rotation {
            list: Playlist {
                output: None,
                sources: ["a", "b", "c"].map(PathBuf::from).to_vec(),
                folder: None,
                settings: Default::default(),
                interval_secs: 300,
                fit: crate::protocol::Fit::Cover,
            },
            index: 0,
            deadline: Instant::now() + Duration::from_secs(300),
            suspended: false,
            failed: HashSet::new(),
            unreadable: false,
        }
    }

    #[test]
    fn each_rotation_entry_retains_its_own_properties_and_look() {
        let mut rotation = rotation();
        let settings = apolline_wall_protocol::playlist::Settings {
            properties: std::collections::BTreeMap::from([(
                "sun".into(),
                serde_json::json!(false),
            )]),
            look: Some(crate::protocol::Look {
                brightness: -0.2,
                speed: 0.5,
                ..Default::default()
            }),
        };
        rotation.list.settings.insert("a".into(), settings.clone());
        let show = rotation.show(None);
        assert_eq!(show.properties, settings.properties);
        assert_eq!(show.look, settings.look);
        rotation.advance(false);
        let plain = rotation.show(Some("DP-1".into()));
        assert!(plain.properties.is_empty());
        assert!(plain.look.is_none());
        assert_eq!(plain.output.as_deref(), Some("DP-1"));
        rotation.advance(true);
        assert_eq!(rotation.show(None), show);
    }

    #[test]
    fn automatic_and_manual_steps_skip_failed_entries_in_both_directions() {
        let mut rotation = rotation();
        assert!(rotation.fail(Path::new("a")));
        assert!(rotation.advance(false));
        assert_eq!(rotation.index, 1);
        assert!(rotation.advance(true));
        assert_eq!(rotation.index, 2);
        assert!(rotation.advance(false));
        assert_eq!(rotation.index, 1);
    }

    #[test]
    fn failures_of_old_requests_do_not_quarantine_the_current_entry() {
        let mut rotation = rotation();
        assert!(!rotation.fail(Path::new("c")));
        assert_eq!(rotation.available(), 3);
        assert!(rotation.fail(Path::new("a")));
        assert!(!rotation.fail(Path::new("a")));
        assert_eq!(rotation.available(), 2);
        assert!(rotation.deadline <= Instant::now());
    }

    #[test]
    fn a_folder_rescan_keeps_the_place_of_what_shows() {
        let folder = std::env::temp_dir().join(format!("wall-rotation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["a.jpg", "c.jpg", "e.jpg"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let mut rotation = rotation();
        rotation.list.folder = Some(folder.clone());
        rotation.list.sources = folder_sources(&folder).unwrap();
        rotation.index = 1;
        rotation.failed.insert(folder.join("e.jpg"));
        std::fs::write(folder.join("b.jpg"), b"").unwrap();
        rotation.rescan().unwrap();
        assert_eq!(rotation.list.sources[rotation.index], folder.join("c.jpg"));
        assert_eq!(rotation.list.sources.len(), 4);
        std::fs::remove_file(folder.join("c.jpg")).unwrap();
        std::fs::remove_file(folder.join("e.jpg")).unwrap();
        rotation.rescan().unwrap();
        assert!(rotation.failed.is_empty());
        assert!(rotation.advance(false));
        assert_eq!(rotation.list.sources[rotation.index], folder.join("a.jpg"));
        std::fs::remove_dir_all(&folder).unwrap();
        assert!(rotation.rescan().is_err());
        assert_eq!(rotation.list.sources.len(), 2);
        // What failed while the folder was away gets another go once it's back.
        rotation.failed.insert(folder.join("a.jpg"));
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["a.jpg", "b.jpg"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        rotation.rescan().unwrap();
        assert!(rotation.failed.is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_shown_wallpaper_gone_from_the_folder_gives_way_even_to_a_single_one() {
        let folder =
            std::env::temp_dir().join(format!("wall-rotation-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["a.jpg", "b.jpg"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let mut rotation = rotation();
        rotation.list.folder = Some(folder.clone());
        rotation.list.sources = folder_sources(&folder).unwrap();
        rotation.index = 0;
        std::fs::remove_file(folder.join("a.jpg")).unwrap();
        assert_eq!(rotation.step(false), Ok(true));
        assert_eq!(rotation.list.sources[rotation.index], folder.join("b.jpg"));
        // Nothing left the folder: one wallpaper stays where it is.
        assert_eq!(rotation.step(false), Ok(false));
        std::fs::write(folder.join("c.jpg"), b"").unwrap();
        rotation.rescan().unwrap();
        rotation.index = 1;
        std::fs::remove_file(folder.join("c.jpg")).unwrap();
        std::fs::write(folder.join("d.jpg"), b"").unwrap();
        // Back from a gone one is the one before it.
        assert_eq!(rotation.step(true), Ok(true));
        assert_eq!(rotation.list.sources[rotation.index], folder.join("b.jpg"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn one_remaining_entry_settles_without_a_repeating_timer() {
        let mut rotation = rotation();
        rotation.fail(Path::new("a"));
        rotation.advance(false);
        rotation.fail(Path::new("b"));
        assert!(rotation.needs_step());
        assert!(rotation.advance(false));
        assert_eq!(rotation.index, 2);
        assert!(!rotation.needs_step());
        assert!(!rotation.advance(false));
        rotation.fail(Path::new("c"));
        assert_eq!(rotation.available(), 0);
        assert!(!rotation.needs_step());
        assert!(!rotation.advance(true));
    }
}
