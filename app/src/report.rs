//! "Not working? Report it": a plain-text file about the wallpaper, this system and what the
//! daemon said lately, with the home folder and the user's name taken out, saved where people
//! find their files.

use std::path::{Path, PathBuf};

use crate::library::Item;
use crate::{Daemon, config};

/// Programs whose presence decides what can be shown.
const PROGRAMS: [&str; 5] = [
    "mpv",
    "ffmpeg",
    "linux-wallpaperengine",
    "steamcmd",
    "steam",
];

pub struct Report<'a> {
    pub item: &'a Item,
    pub daemon: Option<&'a Daemon>,
    pub errors: &'a [String],
}

impl Report<'_> {
    /// Writes the report and says where it went.
    pub fn save(&self) -> Result<PathBuf, String> {
        let home = config::home();
        let user = std::env::var("USER").unwrap_or_default();
        let text = redact(&self.compose(), &home, &user);
        let folder = [home.join("Downloads"), config::cache_dir()]
            .into_iter()
            .find(|folder| folder.is_dir() || std::fs::create_dir_all(folder).is_ok())
            .ok_or("no folder to save the report in")?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let path = folder.join(format!("apolline-wall-report-{stamp}.txt"));
        std::fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(path)
    }

    fn compose(&self) -> String {
        let item = self.item;
        let mut lines = vec![
            format!("apolline-wallpapers {}", env!("CARGO_PKG_VERSION")),
            String::new(),
            "Wallpaper".into(),
            format!("  name: {}", item.name),
            format!("  kind: {}", item.what.label()),
            format!("  path: {}", item.path.display()),
            format!(
                "  from Wallpaper Engine: {}",
                if item.engine { "yes" } else { "no" }
            ),
        ];
        if let Some(id) = &item.workshop {
            lines.push(format!("  Workshop item: {id}"));
        }
        if let Some(why) = item.what.missing() {
            lines.push(format!("  can't be set: {why}"));
        }
        lines.extend(file_facts(&item.path));
        lines.push(String::new());
        lines.push("System".into());
        lines.push(format!("  {}", distribution()));
        for variable in ["XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"] {
            let value = std::env::var(variable).unwrap_or_else(|_| "unset".into());
            lines.push(format!("  {variable}: {value}"));
        }
        for program in PROGRAMS {
            let found = if on_path(program) {
                "installed"
            } else {
                "not found"
            };
            lines.push(format!("  {program}: {found}"));
        }
        lines.push(String::new());
        match self.daemon {
            Some(daemon) => {
                lines.push(format!(
                    "Daemon: running, shader limits {} fps at {:.0}% resolution",
                    daemon.fps,
                    daemon.scale.unwrap_or(1.0) * 100.0
                ));
                for monitor in &daemon.outputs {
                    let showing = monitor
                        .source
                        .as_deref()
                        .map_or("nothing".into(), |source| source.display().to_string());
                    let paused = if monitor.paused { ", paused" } else { "" };
                    lines.push(format!("  {}: {showing}{paused}", monitor.name));
                }
            }
            None => lines.push("Daemon: not running".into()),
        }
        lines.push(String::new());
        lines.push("Recent errors".into());
        if self.errors.is_empty() {
            lines.push("  none".into());
        }
        lines.extend(self.errors.iter().map(|error| format!("  {error}")));
        lines.join("\n") + "\n"
    }
}

/// What the file itself looks like: its size, and for a Wallpaper Engine project what
/// project.json says it is.
fn file_facts(path: &Path) -> Vec<String> {
    let mut facts = Vec::new();
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => facts.push(format!("  size: {} bytes", meta.len())),
        Ok(_) => {}
        Err(err) => facts.push(format!("  unreadable: {err}")),
    }
    let folder = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    let project = folder.and_then(|folder| {
        let text = std::fs::read_to_string(folder.join("project.json")).ok()?;
        serde_json::from_str::<serde_json::Value>(&text).ok()
    });
    if let Some(project) = project {
        let field = |name: &str| {
            project
                .get(name)
                .and_then(|value| value.as_str())
                .unwrap_or("?")
        };
        facts.push(format!(
            "  project type: {}, file: {}",
            field("type"),
            field("file")
        ));
        let packaged = folder.is_some_and(|folder| folder.join("scene.pkg").is_file());
        facts.push(format!(
            "  scene.pkg: {}",
            if packaged { "yes" } else { "no" }
        ));
    }
    facts
}

fn distribution() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Takes the home folder and the user's name out, the name only where it stands as a word,
/// so a report can be posted publicly.
pub fn redact(text: &str, home: &Path, user: &str) -> String {
    let home = home.to_string_lossy();
    let text = if home.len() > 1 {
        text.replace(home.as_ref(), "~")
    } else {
        text.to_owned()
    };
    if user.len() < 2 {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find(user) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + user.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        out.push_str(if word(before) || word(after) {
            user
        } else {
            "<user>"
        });
        rest = &rest[at + user.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_the_wallpaper_the_system_and_what_went_wrong() {
        let item = Item {
            path: PathBuf::from("/home/dan/Videos/rain.mp4"),
            name: "rain".into(),
            what: crate::library::What::Video,
            modified: std::time::SystemTime::UNIX_EPOCH,
            preview: None,
            workshop: Some("2317494988".into()),
            engine: true,
        };
        let errors = ["videos need libmpv".to_owned()];
        let report = Report {
            item: &item,
            daemon: None,
            errors: &errors,
        };
        let text = redact(&report.compose(), Path::new("/home/dan"), "dan");
        for wanted in [
            "kind: Video",
            "path: ~/Videos/rain.mp4",
            "Workshop item: 2317494988",
            "XDG_SESSION_TYPE",
            "linux-wallpaperengine:",
            "Daemon: not running",
            "  videos need libmpv",
        ] {
            assert!(text.contains(wanted), "{wanted} missing from:\n{text}");
        }
        assert!(!text.contains("/home/dan"));
    }

    #[test]
    fn the_home_folder_and_the_user_name_come_out() {
        let text = "path: /home/dan/Pictures/a.jpg\nowner dan, danger stays, /mnt/dan/x";
        assert_eq!(
            redact(text, Path::new("/home/dan"), "dan"),
            "path: ~/Pictures/a.jpg\nowner <user>, danger stays, /mnt/<user>/x"
        );
        assert_eq!(redact("a", Path::new("/"), ""), "a");
    }
}
