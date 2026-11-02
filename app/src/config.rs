use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The app's own settings; what each monitor shows belongs to the daemon.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub folders: Vec<PathBuf>,
    /// Workshop items rated Questionable; Mature ones are never shown.
    #[serde(default)]
    pub questionable: bool,
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

/// Where wallpapers usually live, listed whether or not they exist yet.
pub fn defaults() -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    vec![
        home().join("Pictures/Wallpapers"),
        home().join("Videos/Wallpapers"),
        data.join("apolline-wall/wallpapers"),
        PathBuf::from("/usr/share/apolline-wall/wallpapers"),
    ]
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
