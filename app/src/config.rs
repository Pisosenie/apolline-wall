use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use apolline_wall_protocol::protocol::Look;
use serde::{Deserialize, Serialize};

/// The app's own settings; what each monitor shows belongs to the daemon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub folders: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub favorites: BTreeSet<PathBuf>,
    #[serde(default = "rotation_minutes")]
    pub rotation_minutes: u32,
    /// Workshop items rated Questionable; Mature ones are never shown.
    #[serde(default)]
    pub questionable: bool,
    /// How each wallpaper should look, by its path; plain ones aren't listed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub looks: BTreeMap<String, Look>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub look_presets: BTreeMap<String, Look>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wallpaper_properties: BTreeMap<String, apolline_scene::properties::Values>,
    /// The Steam account steamcmd signs in with; its password is never kept here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_account: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            favorites: BTreeSet::new(),
            rotation_minutes: 5,
            questionable: false,
            looks: BTreeMap::new(),
            look_presets: BTreeMap::new(),
            wallpaper_properties: BTreeMap::new(),
            steam_account: None,
        }
    }
}

impl Config {
    pub fn wallpaper_settings(&self, path: &Path) -> apolline_wall_protocol::playlist::Settings {
        let key = path.to_string_lossy();
        apolline_wall_protocol::playlist::Settings {
            properties: self
                .wallpaper_properties
                .get(key.as_ref())
                .cloned()
                .unwrap_or_default(),
            look: self
                .looks
                .get(key.as_ref())
                .copied()
                .filter(|look| !look.is_plain()),
        }
    }

    pub fn playlist_settings(
        &self,
        sources: &[PathBuf],
    ) -> BTreeMap<PathBuf, apolline_wall_protocol::playlist::Settings> {
        sources
            .iter()
            .filter_map(|path| {
                let settings = self.wallpaper_settings(path);
                (!settings.is_plain()).then(|| (path.clone(), settings))
            })
            .collect()
    }

    /// The usual places, then the ones added in Settings.
    pub fn all_folders(&self) -> Vec<PathBuf> {
        defaults()
            .into_iter()
            .chain(self.folders.iter().cloned())
            .collect()
    }
}

fn rotation_minutes() -> u32 {
    5
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("apolline-wall")
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"))
        .join("apolline-wall")
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
        .join("apolline-wall")
}

/// Where wallpapers usually live, listed whether or not they exist yet.
pub fn defaults() -> Vec<PathBuf> {
    vec![
        home().join("Pictures/Wallpapers"),
        home().join("Videos/Wallpapers"),
        data_dir().join("wallpapers"),
        PathBuf::from("/usr/share/apolline-wall/wallpapers"),
    ]
}

/// The XDG autostart entry that brings the tray up at login.
fn autostart() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("autostart/apolline-wallpapers-tray.desktop")
}

pub fn tray_at_login() -> bool {
    autostart().is_file()
}

pub fn set_tray_at_login(on: bool) -> Result<(), String> {
    let path = autostart();
    let fail = |err: std::io::Error| format!("{}: {err}", path.display());
    if !on {
        return match std::fs::remove_file(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(fail(err)),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    std::fs::write(
        &path,
        "[Desktop Entry]\nType=Application\nName=Wallpapers in the tray\nExec=apolline-wallpapers --tray\nIcon=preferences-desktop-wallpaper\nNoDisplay=true\n",
    )
    .map_err(fail)
}

pub fn expand(text: &str) -> PathBuf {
    match text.trim().strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(text.trim()),
    }
}

pub fn load() -> Config {
    std::fs::read_to_string(config_dir().join("app.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> Result<(), String> {
    let path = config_dir().join("app.toml");
    let fail = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let text = toml::to_string(config).map_err(|err| fail(&err))?;
    std::fs::create_dir_all(config_dir()).map_err(|err| fail(&err))?;
    let fresh = path.with_extension("toml.new");
    std::fs::write(&fresh, text).map_err(|err| fail(&err))?;
    std::fs::rename(&fresh, &path).map_err(|err| fail(&err))
}

pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if !home().as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playlist_snapshot_keeps_only_selected_customizations() {
        let mut config = Config::default();
        let look = Look {
            brightness: -0.2,
            ..Default::default()
        };
        config.looks.insert("/w/a".into(), look);
        config.looks.insert("/w/plain".into(), Look::default());
        config.wallpaper_properties.insert(
            "/w/scene".into(),
            BTreeMap::from([("sun".into(), serde_json::json!(false))]),
        );
        config.wallpaper_properties.insert(
            "/w/outside".into(),
            BTreeMap::from([("sun".into(), serde_json::json!(true))]),
        );
        let sources = ["/w/a", "/w/plain", "/w/scene"].map(PathBuf::from);
        let snapshot = config.playlist_settings(&sources);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[Path::new("/w/a")].look, Some(look));
        assert_eq!(snapshot[Path::new("/w/scene")].properties["sun"], false);
        assert!(!snapshot.contains_key(Path::new("/w/outside")));
        config.wallpaper_properties.clear();
        assert_eq!(snapshot[Path::new("/w/scene")].properties["sun"], false);
    }
}
