use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Show(Show),
    Pause(Pause),
    Limits(Limits),
    Uniforms(Uniforms),
    /// Answered with a `status` event to whoever asked.
    Status {},
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Show {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub source: PathBuf,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default = "default_transition")]
    pub transition_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub paused: bool,
}

/// How hard animated wallpapers may work; both apply to shaders, fields left out stay as they were.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uniforms {
    #[serde(
        default,
        deserialize_with = "accent",
        serialize_with = "hex",
        skip_serializing_if = "Option::is_none"
    )]
    pub accent: Option<[f32; 3]>,
}

fn default_transition() -> u32 {
    400
}

fn accent<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<[f32; 3]>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|text| rgb(&text).ok_or_else(|| D::Error::custom(format!("{text} is not #rrggbb"))))
        .transpose()
}

fn hex<S: Serializer>(accent: &Option<[f32; 3]>, serializer: S) -> Result<S::Ok, S::Error> {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    match accent {
        Some([r, g, b]) => serializer.serialize_str(&format!(
            "#{:02x}{:02x}{:02x}",
            byte(*r),
            byte(*g),
            byte(*b)
        )),
        None => serializer.serialize_none(),
    }
}

pub fn rgb(text: &str) -> Option<[f32; 3]> {
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let channel = |at: usize| {
        u8::from_str_radix(hex.get(at..at + 2)?, 16)
            .ok()
            .map(|value| value as f32 / 255.0)
    };
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Ready {
        output: String,
        source: PathBuf,
        poster: PathBuf,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<PathBuf>,
        message: String,
    },
    Status {
        outputs: Vec<Monitor>,
        fps: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f32>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Monitor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    pub paused: bool,
}

pub fn parse(line: &str) -> Result<Command, String> {
    let command = serde_json::from_str(line).map_err(|err| format!("{err} in {line}"))?;
    match &command {
        Command::Limits(Limits { fps: Some(fps), .. }) if !(1..=240).contains(fps) => {
            Err(format!("fps {fps} is outside 1..240 in {line}"))
        }
        Command::Limits(Limits {
            scale: Some(scale), ..
        }) if !(0.1..=1.0).contains(scale) => {
            Err(format!("scale {scale} is outside 0.1..1 in {line}"))
        }
        _ => Ok(command),
    }
}

pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("apolline-wall.sock")
}

static LISTENERS: Mutex<Vec<UnixStream>> = Mutex::new(Vec::new());

/// Everything emitted from now on also goes to this client, until writing to it fails.
pub fn listen(stream: UnixStream) {
    if let Ok(mut listeners) = LISTENERS.lock() {
        listeners.push(stream);
    }
}

pub fn emit(event: &Event) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
    if let Ok(mut listeners) = LISTENERS.lock() {
        listeners.retain_mut(|stream| writeln!(stream, "{line}").is_ok());
    }
}

pub fn line<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_fills_in_the_defaults() {
        assert_eq!(
            parse(r#"{"show":{"source":"/w/fog.jpg"}}"#),
            Ok(Command::Show(Show {
                output: None,
                source: "/w/fog.jpg".into(),
                fit: Fit::Cover,
                transition_ms: 400,
            }))
        );
        assert_eq!(
            parse(
                r#"{"show":{"output":"DP-1","source":"/w/a.png","fit":"contain","transition_ms":0}}"#
            ),
            Ok(Command::Show(Show {
                output: Some("DP-1".into()),
                source: "/w/a.png".into(),
                fit: Fit::Contain,
                transition_ms: 0,
            }))
        );
    }

    #[test]
    fn pause_without_an_output_means_every_output() {
        assert_eq!(
            parse(r#"{"pause":{"paused":true}}"#),
            Ok(Command::Pause(Pause {
                output: None,
                paused: true,
            }))
        );
    }

    #[test]
    fn unknown_commands_and_typos_are_errors_not_guesses() {
        assert!(parse(r#"{"lights":{"on":true}}"#).is_err());
        assert!(parse(r#"{"show":{"source":"/w/a.png","transition":10}}"#).is_err());
        assert!(parse("not json").unwrap_err().contains("not json"));
    }

    #[test]
    fn limits_and_uniforms_are_checked_on_the_way_in() {
        assert_eq!(
            parse(r#"{"limits":{"fps":10}}"#),
            Ok(Command::Limits(Limits {
                fps: Some(10),
                scale: None,
            }))
        );
        assert!(parse(r#"{"limits":{"fps":0}}"#).is_err());
        assert!(parse(r#"{"limits":{"scale":1.5}}"#).is_err());
        assert_eq!(
            parse(r##"{"uniforms":{"accent":"#ff0000"}}"##),
            Ok(Command::Uniforms(Uniforms {
                accent: Some([1.0, 0.0, 0.0]),
            }))
        );
        assert!(parse(r#"{"uniforms":{"accent":"red"}}"#).is_err());
        assert_eq!(rgb("#86bdf0").map(|c| (c[0] * 255.0).round()), Some(134.0));
        assert_eq!(rgb("#86bdf"), None);
        assert_eq!(rgb("#zzzzzz"), None);
    }

    #[test]
    fn commands_survive_the_trip_through_json() {
        for command in [
            Command::Show(Show {
                output: Some("DP-1".into()),
                source: "/w/fog.jpg".into(),
                fit: Fit::Contain,
                transition_ms: 0,
            }),
            Command::Pause(Pause {
                output: None,
                paused: true,
            }),
            Command::Limits(Limits {
                fps: Some(10),
                scale: None,
            }),
            Command::Uniforms(Uniforms {
                accent: rgb("#ff6a3d"),
            }),
            Command::Status {},
        ] {
            assert_eq!(parse(&line(&command)), Ok(command));
        }
        assert_eq!(line(&Command::Status {}), r#"{"status":{}}"#);
    }

    #[test]
    fn events_are_one_json_object_each() {
        let ready = Event::Ready {
            output: "DP-1".into(),
            source: "/w/fog.jpg".into(),
            poster: "/w/fog.jpg".into(),
        };
        assert_eq!(
            serde_json::to_string(&ready).unwrap(),
            r#"{"ready":{"output":"DP-1","source":"/w/fog.jpg","poster":"/w/fog.jpg"}}"#
        );
        let error = Event::Error {
            output: None,
            source: None,
            message: "bad".into(),
        };
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"error":{"message":"bad"}}"#
        );
    }
}
