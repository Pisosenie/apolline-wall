use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Show(Show),
    Pause(Pause),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Show {
    #[serde(default)]
    pub output: Option<String>,
    pub source: PathBuf,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default = "default_transition")]
    pub transition_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    #[serde(default)]
    pub output: Option<String>,
    pub paused: bool,
}

fn default_transition() -> u32 {
    400
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Ready {
        output: String,
        source: PathBuf,
        poster: PathBuf,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<PathBuf>,
        message: String,
    },
}

pub fn parse(line: &str) -> Result<Command, String> {
    serde_json::from_str(line).map_err(|err| format!("{err} in {line}"))
}

pub fn emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    if let Ok(line) = serde_json::to_string(event) {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
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
        assert!(parse(r#"{"limits":{"fps":30}}"#).is_err());
        assert!(parse(r#"{"show":{"source":"/w/a.png","transition":10}}"#).is_err());
        assert!(parse("not json").unwrap_err().contains("not json"));
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
