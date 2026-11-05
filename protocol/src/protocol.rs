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
    /// Shown whole, with the edges it leaves filled by a blurred, dimmed copy of itself.
    Blur,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Show(Show),
    Playlist(Playlist),
    Step(Step),
    Pause(Pause),
    Policy(crate::power::Policy),
    Session(Session),
    Limits(Limits),
    Uniforms(Uniforms),
    Adjust(Adjust),
    /// Answered with a `status` event to whoever asked.
    Status {},
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Show {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub properties: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default = "default_transition")]
    pub transition_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playlist {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub sources: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub settings: std::collections::BTreeMap<PathBuf, crate::playlist::Settings>,
    pub interval_secs: u64,
    #[serde(default)]
    pub fit: Fit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default)]
    pub previous: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub locked: bool,
}

/// How a wallpaper looks on top of what its file holds: tone, mirroring and pace.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Look {
    /// -1 to 1; 0 leaves it.
    pub brightness: f32,
    /// 0 to 2; 1 leaves it.
    pub contrast: f32,
    /// 0 to 2; 1 leaves it, 0 is grey.
    pub saturation: f32,
    pub flip: bool,
    /// 0.1 to 4, for videos and shaders.
    pub speed: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            flip: false,
            speed: 1.0,
        }
    }
}

impl Look {
    pub fn is_plain(&self) -> bool {
        *self == Look::default()
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        let within = |value: f32, low: f32, high: f32, name: &str| {
            if (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} {value} is outside {low}..{high}"))
            }
        };
        within(self.brightness, -1.0, 1.0, "brightness")?;
        within(self.contrast, 0.0, 2.0, "contrast")?;
        within(self.saturation, 0.0, 2.0, "saturation")?;
        within(self.speed, 0.1, 4.0, "speed")
    }
}

/// Changes the look of what monitors show now. A draft shows at once but isn't remembered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adjust {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub look: Look,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
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
        #[serde(default)]
        power: crate::power::Status,
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
    #[serde(default)]
    pub manual_paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_scene: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub animated_scene: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub playlist: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_status: Option<crate::playlist::Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
}

pub const MAX_COMMAND_BYTES: usize = 2 * 1024 * 1024;

pub fn parse(line: &str) -> Result<Command, String> {
    if line.len() > MAX_COMMAND_BYTES {
        return Err("command exceeds 2 MiB".into());
    }
    let command = serde_json::from_str(line).map_err(|err| format!("{err} in {line}"))?;
    match &command {
        Command::Policy(policy) => policy.check().map(|()| command.clone()),
        Command::Show(show) if show.properties.len() > 512 => {
            Err("a wallpaper may have at most 512 property overrides".into())
        }
        Command::Show(show) if show.properties.values().any(holds_null) => {
            Err("a property override can't be null".into())
        }
        Command::Playlist(list) => list.check().map(|()| command.clone()),
        Command::Limits(Limits { fps: Some(fps), .. }) if !(1..=240).contains(fps) => {
            Err(format!("fps {fps} is outside 1..240 in {line}"))
        }
        Command::Limits(Limits {
            scale: Some(scale), ..
        }) if !(0.1..=1.0).contains(scale) => {
            Err(format!("scale {scale} is outside 0.1..1 in {line}"))
        }
        Command::Show(Show {
            look: Some(look), ..
        })
        | Command::Adjust(Adjust { look, .. }) => look
            .check()
            .map(|()| command.clone())
            .map_err(|problem| format!("{problem} in {line}")),
        _ => Ok(command),
    }
}

/// Whether a value is or holds a JSON `null`, which the TOML state file can't keep.
pub fn holds_null(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Array(items) => items.iter().any(holds_null),
        serde_json::Value::Object(fields) => fields.values().any(holds_null),
        _ => false,
    }
}

pub fn socket_path() -> PathBuf {
    crate::runtime::path().join("control.sock")
}

/// Where daemons from before the socket moved into its own folder listen. One may still be
/// running right after an upgrade, until the session ends.
pub fn old_socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|root| PathBuf::from(root).join("apolline-wall.sock"))
}

/// The socket a client should talk to: the old one only while that daemon is the one there.
pub fn client_socket_path() -> PathBuf {
    let current = socket_path();
    match old_socket_path() {
        Some(old) if !current.exists() && old.exists() => old,
        _ => current,
    }
}

static LISTENERS: Mutex<Vec<(u64, UnixStream)>> = Mutex::new(Vec::new());
static NEXT_LISTENER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct Listener(u64);

impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(mut listeners) = LISTENERS.lock() {
            listeners.retain(|(id, _)| *id != self.0);
        }
    }
}

/// The connection owns its subscription, so closing it releases the writer immediately.
pub fn listen(stream: UnixStream) -> Option<Listener> {
    let mut listeners = LISTENERS.lock().ok()?;
    if listeners.len() >= 64 {
        return None;
    }
    let id = NEXT_LISTENER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    listeners.push((id, stream));
    Some(Listener(id))
}

pub fn emit(event: &Event) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    crate::output::send(&line);
    if let Ok(mut listeners) = LISTENERS.lock() {
        listeners.retain_mut(|(_, stream)| {
            if writeln!(stream, "{line}").is_ok() {
                true
            } else {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                false
            }
        });
    }
}

pub fn line<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_policy_checks_its_limits_and_session_state_is_explicit() {
        assert!(parse(r#"{"policy":{"battery":"reduce","battery_fps":0}}"#).is_err());
        assert!(parse(r#"{"policy":{"battery_scale":1.5}}"#).is_err());
        assert!(parse(r#"{"policy":{"battery":"pause"}}"#).is_ok());
        assert_eq!(
            parse(r#"{"session":{"locked":true}}"#),
            Ok(Command::Session(Session { locked: true }))
        );
    }

    #[test]
    fn subscriptions_release_closed_connections_without_waiting_for_an_event() {
        use std::io::Read;
        for _ in 0..128 {
            let (mut client, server) = UnixStream::pair().unwrap();
            let subscription = listen(server).unwrap();
            drop(subscription);
            client.set_nonblocking(true).unwrap();
            assert_eq!(client.read(&mut [0; 1]).unwrap(), 0);
        }
    }

    #[test]
    fn oversized_commands_are_refused_before_json_parsing() {
        assert_eq!(
            parse(&" ".repeat(MAX_COMMAND_BYTES + 1)).unwrap_err(),
            "command exceeds 2 MiB"
        );
    }

    #[test]
    fn show_fills_in_the_defaults() {
        assert_eq!(
            parse(r#"{"show":{"source":"/w/fog.jpg"}}"#),
            Ok(Command::Show(Show {
                properties: Default::default(),
                output: None,
                source: "/w/fog.jpg".into(),
                fit: Fit::Cover,
                transition_ms: 400,
                look: None,
            }))
        );
        assert_eq!(
            parse(
                r#"{"show":{"output":"DP-1","source":"/w/a.png","fit":"contain","transition_ms":0}}"#
            ),
            Ok(Command::Show(Show {
                properties: Default::default(),
                output: Some("DP-1".into()),
                source: "/w/a.png".into(),
                fit: Fit::Contain,
                transition_ms: 0,
                look: None,
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
    fn a_look_fills_in_what_it_leaves_out_and_stays_in_range() {
        let Ok(Command::Adjust(adjust)) =
            parse(r#"{"adjust":{"look":{"saturation":0.2},"draft":true}}"#)
        else {
            panic!("an adjust")
        };
        assert_eq!(
            adjust.look,
            Look {
                saturation: 0.2,
                ..Look::default()
            }
        );
        assert!(adjust.draft && adjust.output.is_none());
        assert!(Look::default().is_plain());
        assert!(
            parse(r#"{"adjust":{"look":{"speed":9}}}"#)
                .unwrap_err()
                .contains("speed 9")
        );
        assert!(parse(r#"{"show":{"source":"/w/a.jpg","look":{"brightness":-2}}}"#).is_err());
        assert!(parse(r#"{"adjust":{"look":{"glow":1}}}"#).is_err());
    }

    #[test]
    fn commands_survive_the_trip_through_json() {
        for command in [
            Command::Show(Show {
                properties: Default::default(),
                output: Some("DP-1".into()),
                source: "/w/fog.jpg".into(),
                fit: Fit::Contain,
                transition_ms: 0,
                look: Some(Look {
                    flip: true,
                    ..Look::default()
                }),
            }),
            Command::Adjust(Adjust {
                output: None,
                look: Look::default(),
                draft: false,
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
    fn old_monitor_status_defaults_to_static_and_new_animation_flag_round_trips() {
        let mut monitor: Monitor =
            serde_json::from_str(r#"{"name":"DP-1","paused":false}"#).unwrap();
        assert!(!monitor.animated_scene);
        assert!(
            !serde_json::to_value(&monitor)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("animated_scene")
        );
        monitor.native_scene = true;
        monitor.animated_scene = true;
        assert_eq!(
            serde_json::from_str::<Monitor>(&serde_json::to_string(&monitor).unwrap()).unwrap(),
            monitor
        );
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
