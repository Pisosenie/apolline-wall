use serde::{Deserialize, Serialize};

use crate::protocol::{Look, Playlist};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
}

impl Settings {
    pub fn is_plain(&self) -> bool {
        self.properties.is_empty() && self.look.is_none_or(|look| look.is_plain())
    }
}

impl Playlist {
    pub fn check(&self) -> Result<(), String> {
        if !self.sources.is_empty()
            && (!(10..=86400).contains(&self.interval_secs)
                || self.sources.len() > 10000
                || self.sources.iter().any(|path| !path.is_absolute()))
        {
            return Err("a playlist needs absolute paths, at most 10000 wallpapers and an interval of 10–86400 seconds".into());
        }
        let sources: HashSet<_> = self.sources.iter().collect();
        for (source, settings) in &self.settings {
            if !sources.contains(source) {
                return Err(format!(
                    "playlist settings name a source outside the list: {}",
                    source.display()
                ));
            }
            if settings.properties.len() > 512 {
                return Err(format!(
                    "{}: at most 512 property overrides are allowed",
                    source.display()
                ));
            }
            if settings
                .properties
                .values()
                .any(crate::protocol::holds_null)
            {
                return Err(format!(
                    "{}: a property override can't be null",
                    source.display()
                ));
            }
            if let Some(look) = &settings.look {
                look.check()
                    .map_err(|error| format!("{}: {error}", source.display()))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// One-based position in the ordered list, including skipped entries.
    pub position: usize,
    pub total: usize,
    pub skipped: usize,
    pub shared: bool,
    pub suspended: bool,
    pub can_step: bool,
}

impl Status {
    pub fn exhausted(&self) -> bool {
        self.total <= self.skipped
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = if self.shared {
            "Shared playlist"
        } else {
            "Playlist"
        };
        if self.exhausted() {
            return write!(f, "{name} stopped: no playable entries");
        }
        write!(f, "{name} {}/{}", self.position, self.total)?;
        if self.skipped > 0 {
            write!(f, " · {} skipped", self.skipped)?;
        }
        if self.suspended {
            f.write_str(" · paused")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playlist_settings_are_validated_before_commands_are_accepted() {
        use crate::protocol::{Command, line, parse};
        let old = r#"{"playlist":{"sources":["/w/a"],"interval_secs":300}}"#;
        let Command::Playlist(mut list) = parse(old).unwrap() else {
            panic!("expected playlist")
        };
        assert!(list.settings.is_empty());
        let setting = Settings {
            properties: BTreeMap::from([("sun".into(), serde_json::json!(false))]),
            look: Some(Look {
                speed: 0.5,
                ..Default::default()
            }),
        };
        list.settings.insert("/w/a".into(), setting.clone());
        let command = Command::Playlist(list.clone());
        assert_eq!(parse(&line(&command)).unwrap(), command);
        list.settings.insert("/w/outside".into(), setting);
        assert!(list.check().unwrap_err().contains("outside the list"));
        list.settings.remove(std::path::Path::new("/w/outside"));
        list.settings
            .get_mut(std::path::Path::new("/w/a"))
            .unwrap()
            .look
            .as_mut()
            .unwrap()
            .speed = 0.0;
        assert!(parse(&line(&Command::Playlist(list.clone()))).is_err());
        list.settings
            .get_mut(std::path::Path::new("/w/a"))
            .unwrap()
            .look = None;
        list.settings
            .get_mut(std::path::Path::new("/w/a"))
            .unwrap()
            .properties = (0..513)
            .map(|index| (index.to_string(), serde_json::Value::Null))
            .collect();
        assert!(list.check().unwrap_err().contains("512"));
    }

    #[test]
    fn stopped_and_paused_playlists_have_distinct_status() {
        let mut status = Status {
            position: 2,
            total: 3,
            skipped: 1,
            shared: true,
            suspended: true,
            can_step: true,
        };
        assert!(!status.exhausted());
        assert_eq!(
            status.to_string(),
            "Shared playlist 2/3 · 1 skipped · paused"
        );
        status.skipped = 3;
        status.can_step = false;
        assert!(status.exhausted());
        assert_eq!(
            status.to_string(),
            "Shared playlist stopped: no playable entries"
        );
    }

    #[test]
    fn monitors_from_older_daemons_keep_their_playlist_flag() {
        let monitor: crate::protocol::Monitor =
            serde_json::from_str(r#"{"name":"DP-1","paused":false,"playlist":true}"#).unwrap();
        assert!(monitor.playlist);
        assert!(monitor.playlist_status.is_none());
    }
}
