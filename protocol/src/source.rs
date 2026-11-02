use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

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
    /// A Wallpaper Engine project folder, drawn by linux-wallpaperengine.
    Scene,
}

/// What a source would be shown as: a folder is a Wallpaper Engine project, and any file
/// it doesn't recognize is tried as a picture.
pub fn kind(path: &Path) -> Kind {
    if path.is_dir() {
        return Kind::Scene;
    }
    known(path).unwrap_or(Kind::Picture)
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

/// Where the daemon leaves a still of a video or shader it has shown.
pub fn poster_path(source: &Path) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("apolline");
    let _ = std::fs::create_dir_all(&dir);
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dir.join(format!("poster-{:016x}.png", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
