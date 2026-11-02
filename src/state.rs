use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::{self, Adjust, Command, Fit, Limits, Look, Show, Uniforms};

/// What a login comes back to when the daemon runs on its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub source: PathBuf,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
}

impl Saved {
    fn of(show: &Show) -> Self {
        Saved {
            source: show.source.clone(),
            fit: show.fit,
            look: show.look.filter(|look| !look.is_plain()),
        }
    }
}

impl State {
    /// Remembers what a command changes; a pause or a status request is not worth a login.
    pub fn record(&mut self, command: &Command) -> bool {
        match command {
            Command::Show(show) => {
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
                                look,
                                ..default.clone()
                            };
                            self.outputs.insert(name.clone(), saved);
                        }
                        (None, None) => return false,
                    },
                }
                true
            }
            Command::Uniforms(_) | Command::Pause(_) | Command::Status {} => false,
        }
    }

    /// Marks a wallpaper as one that has shown, so a later failure can fall back to it.
    pub fn keep(&mut self, show: &Show) {
        let saved = Saved::of(show);
        match &show.output {
            None => self.wallpaper = Some(saved),
            Some(name) => {
                self.outputs.insert(name.clone(), saved);
            }
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
        let show = |output: Option<&String>, saved: &Saved| {
            Command::Show(Show {
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
        Ok(text) => toml::from_str(&text).map_err(|err| format!("{}: {err}", path.display())),
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

    fn show(output: Option<&str>, source: &str) -> Command {
        Command::Show(Show {
            output: output.map(Into::into),
            source: source.into(),
            fit: Fit::Cover,
            transition_ms: 400,
            look: None,
        })
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
        assert_eq!(state.wallpaper.as_ref().unwrap().look, None);
        let Command::Show(replayed) = &state.commands()[1] else {
            panic!()
        };
        assert_eq!(replayed.look, Some(dim));
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
