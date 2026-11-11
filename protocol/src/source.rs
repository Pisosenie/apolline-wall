use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const PICTURE: [&str; 5] = ["jpg", "jpeg", "png", "webp", "jxl"];

const VIDEO: [&str; 11] = [
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "ogv", "ts", "gif",
];

const SHADER: [&str; 2] = ["frag", "glsl"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Picture,
    Video,
    Shader,
    /// A Wallpaper Engine scene or web project folder.
    Scene,
}

/// What a source would be shown as: a folder is a Wallpaper Engine project, a video one
/// played as its video, and any file it doesn't recognize is tried as a picture.
pub fn kind(path: &Path) -> Kind {
    if path.is_dir() {
        return match project_video(path) {
            Some(_) => Kind::Video,
            None => Kind::Scene,
        };
    }
    known(path).unwrap_or(Kind::Picture)
}

/// The file a Wallpaper Engine video project plays, when the folder is one.
pub fn project_video(folder: &Path) -> Option<PathBuf> {
    #[derive(serde::Deserialize)]
    struct Project {
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        file: String,
    }
    let project: Project = read_project(folder).ok()?;
    if !project.kind.eq_ignore_ascii_case("video") {
        return None;
    }
    project_file(folder, &project.file).filter(|file| known(file) == Some(Kind::Video))
}

/// Only the files a library should list.
pub fn known(path: &Path) -> Option<Kind> {
    let ext = path.extension().and_then(|ext| ext.to_str())?;
    let is = |list: &[&str]| list.iter().any(|known| known.eq_ignore_ascii_case(ext));
    if is(&VIDEO) {
        Some(Kind::Video)
    } else if is(&SHADER) {
        Some(Kind::Shader)
    } else if is(&PICTURE) {
        Some(Kind::Picture)
    } else {
        None
    }
}

/// Reads project metadata with the same limit for library scans and renderer posters.
pub fn read_project<T: serde::de::DeserializeOwned>(folder: &Path) -> Result<T, String> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let path = folder.join("project.json");
    let fail = |error| format!("{}: {error}", path.display());
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(&path).map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(format!(
            "{} must be a regular file no larger than 8 MiB",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 > LIMIT {
        return Err(format!("{} grew past 8 MiB", path.display()));
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

/// Resolves metadata's local files inside the project, including symlink targets.
pub fn project_file(folder: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let root = folder.canonicalize().ok()?;
    let file = folder.join(path);
    let resolved = file.canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(file)
}

/// Where the daemon leaves a still of a video or shader it has shown.
pub fn poster_path(source: &Path) -> PathBuf {
    let dir = crate::runtime::path();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dir.join(format!("poster-{:016x}.png", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_metadata_is_bounded_and_files_stay_inside_the_folder() {
        let folder = std::env::temp_dir().join(format!("wall-metadata-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let metadata = folder.join("project.json");
        std::fs::write(&metadata, br#"{"preview":"poster.png"}"#).unwrap();
        let project: serde_json::Value = read_project(&folder).unwrap();
        assert_eq!(project["preview"], "poster.png");
        std::fs::write(folder.join("poster.png"), b"picture").unwrap();
        assert_eq!(
            project_file(&folder, "poster.png"),
            Some(folder.join("poster.png"))
        );
        assert!(project_file(&folder, "../project.json").is_none());
        assert!(project_file(&folder, metadata.to_str().unwrap()).is_none());
        assert!(project_file(&folder, "").is_none());
        #[cfg(unix)]
        {
            let link = folder.join("outside");
            let outside = folder.with_extension("outside");
            std::fs::write(&outside, b"outside project").unwrap();
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(project_file(&folder, "outside").is_none());
            std::fs::remove_file(link).unwrap();
            std::fs::remove_file(outside).unwrap();
            let inside = folder.join("inside");
            std::os::unix::fs::symlink(folder.join("poster.png"), &inside).unwrap();
            assert_eq!(project_file(&folder, "inside"), Some(inside));
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&metadata)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert!(
            read_project::<serde_json::Value>(&folder)
                .unwrap_err()
                .contains("8 MiB")
        );
        std::fs::remove_file(&metadata).unwrap();
        std::fs::create_dir(&metadata).unwrap();
        assert!(read_project::<serde_json::Value>(&folder).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_video_project_folder_plays_its_video_and_a_scene_folder_stays_a_scene() {
        let dir = std::env::temp_dir().join(format!("wall-video-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"").unwrap();
        std::fs::write(
            dir.join("project.json"),
            r#"{"type":"Video","file":"clip.mp4"}"#,
        )
        .unwrap();
        assert_eq!(kind(&dir), Kind::Video);
        assert_eq!(project_video(&dir), Some(dir.join("clip.mp4")));
        std::fs::write(
            dir.join("project.json"),
            r#"{"type":"video","file":"../etc/passwd"}"#,
        )
        .unwrap();
        assert_eq!(kind(&dir), Kind::Scene);
        std::fs::write(
            dir.join("project.json"),
            r#"{"type":"scene","file":"scene.json"}"#,
        )
        .unwrap();
        assert_eq!(kind(&dir), Kind::Scene);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sources_are_told_apart_by_extension() {
        assert_eq!(kind(Path::new("/w/sea.MP4")), Kind::Video);
        assert_eq!(kind(Path::new("/w/loop.gif")), Kind::Video);
        assert_eq!(kind(Path::new("/w/waves.frag")), Kind::Shader);
        assert_eq!(kind(Path::new("/w/fog.jpg")), Kind::Picture);
        assert_eq!(kind(Path::new("/w/noext")), Kind::Picture);
        assert_eq!(known(Path::new("/w/notes.txt")), None);
        assert_eq!(known(Path::new("/w/noext")), None);
        assert_eq!(kind(&std::env::temp_dir()), Kind::Scene);
    }
}
