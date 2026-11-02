use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

use crate::config;
use crate::library::{Item, What};

/// Wallpaper Engine's Steam app id; its Workshop downloads land in a folder with this name.
const APP: &str = "431960";

#[derive(Deserialize)]
struct Project {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    preview: String,
}

/// Every place Steam keeps Wallpaper Engine's wallpapers: Workshop subscriptions and the
/// projects that ship with it or that you made, in every Steam library on this machine.
pub fn folders() -> Vec<PathBuf> {
    let home = config::home();
    let roots = [
        ".local/share/Steam",
        ".steam/steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
    ]
    .map(|root| home.join(root));
    folders_in(&libraries(&roots))
}

fn libraries(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for root in roots {
        let listed = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf"))
            .map(|vdf| paths_in(&vdf))
            .unwrap_or_default();
        for library in std::iter::once(root.clone()).chain(listed) {
            let canonical = library.canonicalize().unwrap_or(library);
            if !found.contains(&canonical) {
                found.push(canonical);
            }
        }
    }
    found
}

/// The `"path" "..."` lines of Steam's libraryfolders.vdf.
fn paths_in(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter(|line| line.trim_start().starts_with("\"path\""))
        .filter_map(|line| line.rsplit('"').nth(1))
        .map(|path| PathBuf::from(path.replace("\\\\", "\\")))
        .collect()
}

fn folders_in(libraries: &[PathBuf]) -> Vec<PathBuf> {
    libraries
        .iter()
        .flat_map(|library| {
            let apps = library.join("steamapps");
            [
                apps.join("workshop/content").join(APP),
                apps.join("common/wallpaper_engine/projects/defaultprojects"),
                apps.join("common/wallpaper_engine/projects/myprojects"),
            ]
        })
        .filter(|folder| folder.is_dir())
        .collect()
}

/// Every wallpaper in one of those folders, each a directory with a project.json.
pub fn wallpapers(folder: &Path) -> Vec<Item> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| read(&entry.path()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn read(dir: &Path) -> Option<Item> {
    let project_file = dir.join("project.json");
    let project: Project =
        serde_json::from_str(&std::fs::read_to_string(&project_file).ok()?).ok()?;
    let what = match project.kind.to_lowercase().as_str() {
        "video" => What::Video,
        "scene" => What::Scene,
        "web" => What::Web,
        "application" => What::Program,
        _ => return None,
    };
    let path = match what {
        What::Video => dir.join(&project.file),
        _ => dir.to_path_buf(),
    };
    if what == What::Video && !path.is_file() {
        return None;
    }
    let folder_name = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let title = project.title.trim();
    Some(Item {
        name: if title.is_empty() { folder_name } else { title }.to_owned(),
        what,
        modified: project_file
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH),
        preview: Some(dir.join(&project.preview))
            .filter(|preview| !project.preview.is_empty() && preview.is_file()),
        workshop: Some(folder_name.to_owned())
            .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())),
        engine: true,
        path,
    })
}

/// Whether Steam has finished downloading a Workshop item into any library.
pub fn downloaded(id: &str) -> bool {
    folders()
        .iter()
        .any(|folder| folder.join(id).join("project.json").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(root: &Path, id: &str, json: &str, files: &[&str]) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project.json"), json).unwrap();
        for file in files {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        dir
    }

    #[test]
    fn workshop_items_become_library_items() {
        let root = std::env::temp_dir().join(format!("apolline-engine-{}", std::process::id()));
        let content = root.join("steamapps/workshop/content").join(APP);
        project(
            &content,
            "2345678",
            r#"{"title":"Neon rain","type":"Video","file":"rain.mp4","preview":"preview.gif"}"#,
            &["rain.mp4", "preview.gif"],
        );
        project(
            &content,
            "1111",
            r#"{"title":" Forest ","type":"scene","file":"scene.json","preview":"preview.jpg"}"#,
            &[],
        );
        project(
            &content,
            "2222",
            r#"{"type":"video","file":"gone.mp4"}"#,
            &[],
        );
        project(&content, "3333", "not json", &[]);
        assert_eq!(
            folders_in(std::slice::from_ref(&root)),
            vec![content.clone()]
        );
        let mut items = wallpapers(&content);
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let found: Vec<(&str, What, bool)> = items
            .iter()
            .map(|item| (item.name.as_str(), item.what, item.preview.is_some()))
            .collect();
        assert_eq!(
            found,
            [
                ("Forest", What::Scene, false),
                ("Neon rain", What::Video, true)
            ]
        );
        assert!(items[1].path.ends_with("2345678/rain.mp4"));
        assert_eq!(items[1].workshop.as_deref(), Some("2345678"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn library_folders_come_from_the_vdf() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/me/.local/share/Steam"
		"label"		""
	}
	"1"
	{
		"path"		"/mnt/games/SteamLibrary"
	}
}"#;
        assert_eq!(
            paths_in(vdf),
            [
                PathBuf::from("/home/me/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary")
            ]
        );
    }
}
