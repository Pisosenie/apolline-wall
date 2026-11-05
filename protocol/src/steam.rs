//! Steam libraries shared by the wallpaper browser and the scene loader.

use std::path::{Path, PathBuf};

pub fn libraries(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
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

pub fn roots(home: &Path) -> [PathBuf; 3] {
    [
        ".local/share/Steam",
        ".steam/steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
    ]
    .map(|root| home.join(root))
}

/// Prefer assets beside the project, then other Steam libraries and steamcmd's install.
pub fn assets(project: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("APOLLINE_WALL_ASSETS") {
        candidates.push(PathBuf::from(path));
    }
    for parent in project.ancestors() {
        if parent.file_name().is_some_and(|name| name == "steamapps") {
            candidates.push(parent.join("common/wallpaper_engine/assets"));
        }
        if parent
            .file_name()
            .is_some_and(|name| name == "wallpaper_engine")
        {
            candidates.push(parent.join("assets"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.extend(
            libraries(&roots(&home))
                .into_iter()
                .map(|library| library.join("steamapps/common/wallpaper_engine/assets")),
        );
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        candidates
            .push(data.join("apolline-wall/steamcmd/steamapps/common/wallpaper_engine/assets"));
    }
    candidates.into_iter().find(|path| path.is_dir())
}

/// The `"path" "..."` lines of Steam's libraryfolders.vdf.
pub fn paths_in(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter(|line| line.trim_start().starts_with("\"path\""))
        .filter_map(|line| line.rsplit('"').nth(1))
        .map(|path| PathBuf::from(path.replace("\\\\", "\\")))
        .collect()
}
