use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::{self, Adjust, Command, Fit, Limits, Look, Show, Uniforms};

/// What a login comes back to when the daemon runs on its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default)]
    pub power_policy: apolline_wall_protocol::power::Policy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Saved>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, Saved>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub playlists: BTreeMap<String, protocol::Playlist>,
    /// Which monitor each connector last had, by what the compositor calls it (make, model
    /// and usually a serial), so its settings follow it to another connector.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub monitors: BTreeMap<String, String>,
    /// How many runs in a row each wallpaper was loading or just showing when the daemon died.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub crashes: BTreeMap<String, u32>,
}

/// Connectors remembered for monitors: a desk's worth and some docks, not a history.
const MONITORS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follows_default: bool,
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
}

impl Saved {
    fn of(show: &Show) -> Self {
        Saved {
            follows_default: false,
            source: show.source.clone(),
            properties: show.properties.clone(),
            fit: show.fit,
            look: show.look.filter(|look| !look.is_plain()),
        }
    }
}

impl State {
    /// Where a monitor showing up on `connector` was before: the one connector that last had
    /// it, now unplugged, while `connector` has nothing of its own. Two monitors that look the
    /// same can't be told apart, so neither moves.
    pub fn moved(&self, connector: &str, identity: &str, connected: &[&str]) -> Option<String> {
        if identity.trim().is_empty()
            || self.outputs.contains_key(connector)
            || self.playlists.contains_key(connector)
        {
            return None;
        }
        let mut before = self
            .monitors
            .iter()
            .filter(|(seen, known)| known.as_str() == identity && seen.as_str() != connector);
        let (old, _) = before.next()?;
        (before.next().is_none() && !connected.contains(&old.as_str())).then(|| old.clone())
    }

    /// Stops showing a wallpaper anywhere at the next login.
    pub fn forget(&mut self, source: &Path) {
        if self
            .wallpaper
            .as_ref()
            .is_some_and(|saved| saved.source == source)
        {
            self.wallpaper = None;
        }
        self.outputs.retain(|_, saved| saved.source != source);
    }

    /// Moves a connector's wallpaper and playlist to another.
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(saved) = self.outputs.remove(old) {
            self.outputs.insert(new.to_owned(), saved);
        }
        if let Some(mut playlist) = self.playlists.remove(old) {
            playlist.output = Some(new.to_owned());
            self.playlists.insert(new.to_owned(), playlist);
        }
        self.monitors.remove(old);
    }

    /// Notes which monitor a connector has now; says whether that's news.
    pub fn seen(&mut self, connector: &str, identity: &str) -> bool {
        if self.monitors.get(connector).map(String::as_str) == Some(identity) {
            return false;
        }
        if !self.monitors.contains_key(connector) && self.monitors.len() >= MONITORS {
            let unused = self
                .monitors
                .keys()
                .find(|seen| {
                    !self.outputs.contains_key(*seen) && !self.playlists.contains_key(*seen)
                })
                .cloned();
            match unused {
                Some(seen) => {
                    self.monitors.remove(&seen);
                }
                None => return false,
            }
        }
        self.monitors
            .insert(connector.to_owned(), identity.to_owned());
        true
    }

    /// Remembers what a command changes; a pause or a status request is not worth a login.
    pub fn record(&mut self, command: &Command) -> bool {
        match command {
            Command::Policy(policy) => {
                self.power_policy = policy.clone();
                true
            }
            Command::Playlist(list) => {
                let key = list.output.clone().unwrap_or_default();
                if list.output.is_none() {
                    self.playlists.clear();
                    if !list.sources.is_empty() {
                        self.outputs.clear();
                    }
                }
                if list.sources.is_empty() {
                    self.playlists.remove(&key);
                } else {
                    if let Some(saved) = self.outputs.get_mut(&key) {
                        saved.follows_default = false;
                    }
                    self.playlists.insert(key, list.clone());
                }
                true
            }
            Command::Show(show) => {
                match &show.output {
                    None => self.playlists.clear(),
                    Some(output) => {
                        self.playlists.remove(output);
                    }
                }
                let saved = Saved::of(show);
                match &show.output {
                    None => {
                        self.outputs.clear();
                        self.wallpaper = Some(saved);
                    }
                    Some(name) => {
                        self.outputs.insert(name.clone(), saved);
                    }
                }
                true
            }
            Command::Limits(limits) => {
                self.fps = limits.fps.or(self.fps);
                self.scale = limits.scale.or(self.scale);
                limits.fps.is_some() || limits.scale.is_some()
            }
            Command::Uniforms(Uniforms {
                accent: Some(accent),
            }) => {
                self.accent = Some(hex(*accent));
                true
            }
            Command::Adjust(Adjust { draft: true, .. }) => false,
            Command::Adjust(Adjust { output, look, .. }) => {
                let look = Some(*look).filter(|look| !look.is_plain());
                match output {
                    None => {
                        for saved in self.wallpaper.iter_mut().chain(self.outputs.values_mut()) {
                            saved.look = look;
                        }
                    }
                    Some(name) => match (self.outputs.get_mut(name), &self.wallpaper) {
                        (Some(saved), _) => saved.look = look,
                        (None, Some(default)) => {
                            let saved = Saved {
                                follows_default: true,
                                look,
                                ..default.clone()
                            };
                            self.outputs.insert(name.clone(), saved);
                        }
                        (None, None) => return false,
                    },
                }
                self.outputs.retain(|_, saved| {
                    !saved.follows_default
                        || self.wallpaper.as_ref().is_none_or(|default| {
                            default.look.unwrap_or_default() != saved.look.unwrap_or_default()
                        })
                });
                true
            }
            Command::Session(_)
            | Command::Step(_)
            | Command::Uniforms(_)
            | Command::Pause(_)
            | Command::Status {} => false,
        }
    }

    /// Marks a wallpaper as one that has shown, so a later failure can fall back to it.
    pub fn keep(&mut self, show: &Show) {
        let saved = Saved::of(show);
        match &show.output {
            None => {
                self.outputs.retain(|_, previous| {
                    !previous.follows_default || previous.source == saved.source
                });
                self.wallpaper = Some(saved);
            }
            Some(name) => {
                self.outputs.insert(name.clone(), saved);
            }
        }
    }

    /// A rendered draft is not a committed fallback, even if a resize loaded it again.
    pub fn confirm(&mut self, show: &Show, desired: &State) {
        let explicit = show
            .output
            .as_ref()
            .and_then(|name| desired.outputs.get(name));
        let expected = explicit
            .or(desired.wallpaper.as_ref())
            .filter(|saved| saved.source == show.source);
        let confirmed = Show {
            look: expected.map_or(show.look, |saved| saved.look),
            ..show.clone()
        };
        self.keep(&confirmed);
        if let Some(name) = &show.output
            && let Some(saved) = self.outputs.get_mut(name)
        {
            saved.follows_default =
                expected.is_some_and(|saved| saved.follows_default || explicit.is_none());
        }
    }

    /// Takes back a wallpaper that failed, returning to the last one that showed.
    pub fn undo(&mut self, show: &Show, good: &State) -> bool {
        let failed = |saved: Option<&Saved>| saved.is_some_and(|saved| saved.source == show.source);
        match &show.output {
            None if failed(self.wallpaper.as_ref()) => {
                self.wallpaper = good.wallpaper.clone();
                true
            }
            Some(name) if failed(self.outputs.get(name)) => {
                match good.outputs.get(name) {
                    Some(saved) => self.outputs.insert(name.clone(), saved.clone()),
                    None => self.outputs.remove(name),
                };
                true
            }
            _ => false,
        }
    }

    pub fn commands(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        if self.power_policy != Default::default() {
            commands.push(Command::Policy(self.power_policy.clone()));
        }
        if self.fps.is_some() || self.scale.is_some() {
            commands.push(Command::Limits(Limits {
                fps: self.fps,
                scale: self.scale,
            }));
        }
        if let Some(accent) = self.accent.as_deref().and_then(protocol::rgb) {
            commands.push(Command::Uniforms(Uniforms {
                accent: Some(accent),
            }));
        }
        commands.extend(self.playlists.values().cloned().map(Command::Playlist));
        let show = |output: Option<&String>, saved: &Saved| {
            Command::Show(Show {
                properties: saved.properties.clone(),
                output: output.cloned(),
                source: saved.source.clone(),
                fit: saved.fit,
                transition_ms: 0,
                look: saved.look,
            })
        };
        commands.extend(self.wallpaper.iter().map(|saved| show(None, saved)));
        commands.extend(
            self.outputs
                .iter()
                .map(|(name, saved)| show(Some(name), saved)),
        );
        commands
    }
}

fn hex([r, g, b]: [f32; 3]) -> String {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

pub fn path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("apolline-wall").join("state.toml"))
}

pub fn load(path: &Path) -> Result<State, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let state: State =
                toml::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
            state
                .power_policy
                .check()
                .map_err(|err| format!("{}: {err}", path.display()))?;
            for list in state.playlists.values() {
                protocol::parse(&protocol::line(&Command::Playlist(list.clone())))
                    .map_err(|err| format!("{}: {err}", path.display()))?;
            }
            Ok(state)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(err) => Err(format!("{}: {err}", path.display())),
    }
}

pub fn save(path: &Path, state: &State) -> Result<(), String> {
    let fail = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let text = toml::to_string(state).map_err(|err| fail(&err))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| fail(&err))?;
    }
    let fresh = path.with_extension("toml.new");
    std::fs::write(&fresh, text).map_err(|err| fail(&err))?;
    std::fs::rename(&fresh, path).map_err(|err| fail(&err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Pause;

    #[test]
    fn a_forgotten_wallpaper_leaves_every_monitor_that_had_it() {
        let mut state = State::default();
        state.record(&show(None, "/w/crash.frag"));
        state.record(&show(Some("DP-1"), "/w/crash.frag"));
        state.record(&show(Some("DP-2"), "/w/fine.jpg"));
        state.forget(Path::new("/w/crash.frag"));
        assert!(state.wallpaper.is_none());
        assert!(!state.outputs.contains_key("DP-1"));
        assert_eq!(state.outputs["DP-2"].source, PathBuf::from("/w/fine.jpg"));
    }

    #[test]
    fn a_monitor_takes_its_wallpaper_and_playlist_to_another_connector() {
        let mut state = State::default();
        state.record(&show(Some("DP-1"), "/w/dunes.jpg"));
        state.seen("DP-1", "Dell Inc. DELL U2720Q 7XYZ123");
        state.seen("HDMI-A-1", "LG Electronics 27GL850");
        // Unplugged from DP-1, back on DP-2.
        let moved = state.moved("DP-2", "Dell Inc. DELL U2720Q 7XYZ123", &["HDMI-A-1"]);
        assert_eq!(moved.as_deref(), Some("DP-1"));
        state.rename("DP-1", "DP-2");
        state.seen("DP-2", "Dell Inc. DELL U2720Q 7XYZ123");
        assert_eq!(state.outputs["DP-2"].source, PathBuf::from("/w/dunes.jpg"));
        assert!(!state.outputs.contains_key("DP-1"));
        assert!(!state.monitors.contains_key("DP-1"));
    }

    #[test]
    fn nothing_moves_when_the_monitor_could_be_another_or_is_still_plugged_in() {
        let mut state = State::default();
        state.record(&show(Some("DP-1"), "/w/a.jpg"));
        state.seen("DP-1", "Same Model");
        state.seen("DP-3", "Same Model");
        assert_eq!(state.moved("DP-2", "Same Model", &[]), None);
        state.monitors.remove("DP-3");
        assert_eq!(state.moved("DP-2", "Same Model", &["DP-1"]), None);
        assert_eq!(state.moved("DP-2", "", &[]), None);
        state.record(&show(Some("DP-2"), "/w/b.jpg"));
        assert_eq!(state.moved("DP-2", "Same Model", &[]), None);
    }

    fn show(output: Option<&str>, source: &str) -> Command {
        Command::Show(Show {
            properties: Default::default(),
            output: output.map(Into::into),
            source: source.into(),
            fit: Fit::Cover,
            transition_ms: 400,
            look: None,
        })
    }

    #[test]
    fn a_failed_manual_choice_restores_the_previous_assignment_metadata() {
        let mut desired = State::default();
        desired.record(&show(None, "/w/a"));
        let dim = Look {
            brightness: -0.3,
            ..Default::default()
        };
        desired.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: dim,
            draft: false,
        }));
        let Command::Show(mut rendered) = show(Some("DP-1"), "/w/a") else {
            unreachable!()
        };
        rendered.look = Some(dim);
        let mut good = State::default();
        good.confirm(&rendered, &desired);
        let Command::Show(broken) = show(Some("DP-1"), "/w/broken") else {
            unreachable!()
        };
        desired.record(&Command::Show(broken.clone()));
        assert!(!desired.outputs["DP-1"].follows_default);
        assert!(desired.undo(&broken, &good));
        assert!(desired.outputs["DP-1"].follows_default);
        assert_eq!(desired.outputs["DP-1"].source, Path::new("/w/a"));
        assert_eq!(desired.outputs["DP-1"].look, Some(dim));
    }

    #[test]
    fn inherited_look_survives_restart_without_becoming_a_manual_assignment() {
        let mut state = State::default();
        state.record(&show(None, "/w/a"));
        let original = state.clone();
        let dim = Look {
            brightness: -0.3,
            ..Default::default()
        };
        assert!(!state.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: dim,
            draft: true
        })));
        assert_eq!(state, original);
        state.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: dim,
            draft: false,
        }));
        let decoded: State = toml::from_str(&toml::to_string(&state).unwrap()).unwrap();
        assert!(decoded.outputs["DP-1"].follows_default);
        assert_eq!(decoded.outputs["DP-1"].look, Some(dim));
        assert_eq!(decoded.wallpaper.as_ref().unwrap().look, None);
        state.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: Look::default(),
            draft: false,
        }));
        assert!(state.outputs.is_empty());
    }

    #[test]
    fn rotation_removes_inherited_looks_but_keeps_explicit_wallpapers() {
        let mut state = State::default();
        state.record(&show(None, "/w/a"));
        state.record(&show(Some("DP-2"), "/w/manual"));
        state.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: Look {
                speed: 0.5,
                ..Default::default()
            },
            draft: false,
        }));
        let Command::Show(next) = show(None, "/w/b") else {
            unreachable!()
        };
        state.keep(&next);
        assert!(!state.outputs.contains_key("DP-1"));
        assert_eq!(state.outputs["DP-2"].source, Path::new("/w/manual"));
        assert!(!state.outputs["DP-2"].follows_default);
    }

    #[test]
    fn a_rendered_draft_never_becomes_the_committed_fallback_look() {
        let mut desired = State::default();
        desired.record(&show(None, "/w/a"));
        let Command::Show(mut rendered) = show(Some("DP-1"), "/w/a") else {
            unreachable!()
        };
        rendered.look = Some(Look {
            brightness: -0.8,
            ..Default::default()
        });
        let mut good = State::default();
        good.confirm(&rendered, &desired);
        assert_eq!(good.outputs["DP-1"].look, None);
        assert!(good.outputs["DP-1"].follows_default);
        desired.record(&Command::Adjust(Adjust {
            output: Some("DP-1".into()),
            look: rendered.look.unwrap(),
            draft: false,
        }));
        good.confirm(&rendered, &desired);
        assert_eq!(good.outputs["DP-1"].look, rendered.look);
        assert!(good.outputs["DP-1"].follows_default);
    }

    #[test]
    fn automatic_policy_persists_but_session_events_do_not() {
        let mut state = State::default();
        let policy = apolline_wall_protocol::power::Policy {
            battery: apolline_wall_protocol::power::BatteryMode::Pause,
            ..Default::default()
        };
        assert!(state.record(&Command::Policy(policy.clone())));
        assert!(!state.record(&Command::Session(protocol::Session { locked: true })));
        let decoded: State = toml::from_str(&toml::to_string(&state).unwrap()).unwrap();
        assert_eq!(decoded.commands(), vec![Command::Policy(policy)]);
    }

    #[test]
    fn scene_properties_survive_state_serialization_and_replay() {
        let mut state = State::default();
        let Command::Show(mut show) = show(Some("DP-1"), "/w/scene") else {
            unreachable!()
        };
        show.properties
            .insert("visible".into(), serde_json::json!(false));
        show.properties
            .insert("opacity".into(), serde_json::json!(0.25));
        state.record(&Command::Show(show.clone()));
        let decoded: State = toml::from_str(&toml::to_string(&state).unwrap()).unwrap();
        let Command::Show(replayed) = decoded.commands().pop().unwrap() else {
            unreachable!()
        };
        assert_eq!(replayed.properties, show.properties);
    }

    #[test]
    fn a_login_replays_the_default_then_each_monitor_at_once() {
        let mut state = State::default();
        assert!(state.record(&show(Some("DP-1"), "/w/old.jpg")));
        assert!(state.record(&show(None, "/w/fog.jpg")));
        assert!(state.record(&show(Some("HDMI-A-1"), "/w/sea.mp4")));
        assert!(state.record(&Command::Limits(Limits {
            fps: Some(20),
            scale: None,
        })));
        assert!(!state.record(&Command::Pause(Pause {
            output: None,
            paused: true,
        })));
        let replay = state.commands();
        assert_eq!(replay.len(), 3);
        assert!(matches!(
            &replay[0],
            Command::Limits(Limits { fps: Some(20), .. })
        ));
        let Command::Show(first) = &replay[1] else {
            panic!("{replay:?}")
        };
        assert_eq!((first.output.as_deref(), first.transition_ms), (None, 0));
        let Command::Show(second) = &replay[2] else {
            panic!("{replay:?}")
        };
        assert_eq!(second.output.as_deref(), Some("HDMI-A-1"));
    }

    #[test]
    fn a_wallpaper_that_fails_gives_way_to_the_last_one_that_showed() {
        let (mut state, mut good) = (State::default(), State::default());
        let fog = show(None, "/w/fog.jpg");
        let broken = show(None, "/w/broken.frag");
        let Command::Show(fog_show) = &fog else {
            unreachable!()
        };
        let Command::Show(broken_show) = &broken else {
            unreachable!()
        };
        state.record(&fog);
        good.keep(fog_show);
        state.record(&broken);
        assert!(state.undo(broken_show, &good));
        assert_eq!(
            state.wallpaper.as_ref().map(|s| s.source.as_path()),
            Some(Path::new("/w/fog.jpg"))
        );
        assert!(!state.undo(broken_show, &good));
        let odd = show(Some("DP-1"), "/w/odd.png");
        let Command::Show(odd_show) = &odd else {
            unreachable!()
        };
        state.record(&odd);
        assert!(state.undo(odd_show, &good));
        assert!(state.outputs.is_empty());
    }

    #[test]
    fn a_saved_look_follows_its_monitor_and_drafts_are_forgotten() {
        let mut state = State::default();
        state.record(&show(None, "/w/fog.jpg"));
        let dim = Look {
            brightness: -0.3,
            ..Look::default()
        };
        let adjust = |output: Option<&str>, draft| {
            Command::Adjust(Adjust {
                output: output.map(Into::into),
                look: dim,
                draft,
            })
        };
        assert!(!state.record(&adjust(None, true)));
        assert_eq!(state.wallpaper.as_ref().unwrap().look, None);
        assert!(state.record(&adjust(Some("DP-1"), false)));
        assert_eq!(state.outputs["DP-1"].look, Some(dim));
        assert_eq!(state.outputs["DP-1"].source, Path::new("/w/fog.jpg"));
        assert!(state.outputs["DP-1"].follows_default);
        assert_eq!(state.wallpaper.as_ref().unwrap().look, None);
        let Command::Show(replayed) = &state.commands()[1] else {
            panic!()
        };
        assert_eq!(replayed.look, Some(dim));
    }

    #[test]
    fn playlists_restore_before_wallpapers_and_manual_choices_stop_rotation() {
        let mut state = State::default();
        state.record(&show(Some("DP-1"), "/w/a.jpg"));
        let list = protocol::Playlist {
            output: Some("DP-1".into()),
            sources: vec!["/w/a.jpg".into(), "/w/b.jpg".into()],
            settings: BTreeMap::from([(
                "/w/a.jpg".into(),
                apolline_wall_protocol::playlist::Settings {
                    properties: BTreeMap::from([("visible".into(), serde_json::json!(false))]),
                    look: Some(Look {
                        speed: 0.5,
                        ..Default::default()
                    }),
                },
            )]),
            interval_secs: 300,
            fit: Fit::Cover,
        };
        state.record(&Command::Playlist(list.clone()));
        assert_eq!(state.commands().first(), Some(&Command::Playlist(list)));
        let text = toml::to_string(&state).unwrap();
        assert_eq!(toml::from_str::<State>(&text).unwrap(), state);
        state.record(&show(Some("DP-1"), "/w/c.jpg"));
        assert!(state.playlists.is_empty());
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("apolline-wall-state-{}", std::process::id()));
        let path = dir.join("state.toml");
        assert_eq!(load(&path), Ok(State::default()));
        let mut state = State::default();
        state.record(&show(None, "/w/fog.jpg"));
        state.record(&show(Some("DP-1"), "/w/hills.frag"));
        state.record(&Command::Uniforms(Uniforms {
            accent: protocol::rgb("#ff6a3d"),
        }));
        save(&path, &state).unwrap();
        assert_eq!(load(&path), Ok(state));
        std::fs::write(&path, "wallpaper = 3\n").unwrap();
        assert!(load(&path).unwrap_err().contains("state.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
