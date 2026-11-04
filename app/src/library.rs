use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use apolline_wall_protocol::source::{self, Kind};
use iced::widget::image::Handle;

use crate::{config, engine};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What gets shown; for the kinds that can't be shown yet, the wallpaper's folder.
    pub path: PathBuf,
    pub name: String,
    pub what: What,
    pub modified: SystemTime,
    /// A still that comes with the wallpaper, as Wallpaper Engine's do.
    pub preview: Option<PathBuf>,
    pub workshop: Option<String>,
    pub engine: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Picture,
    Video,
    Shader,
    Scene,
    Web,
    Program,
}

impl What {
    pub fn label(self) -> &'static str {
        match self {
            What::Picture => "Picture",
            What::Video => "Video",
            What::Shader => "Shader",
            What::Scene => "Scene",
            What::Web => "Web",
            What::Program => "Program",
        }
    }

    /// Scenes are drawn by another program, which takes no look of ours.
    pub fn takes_a_look(self) -> bool {
        matches!(self, What::Picture | What::Video | What::Shader)
    }

    /// Why it can't be set yet, for the kinds only Wallpaper Engine itself draws.
    pub fn missing(self) -> Option<&'static str> {
        match self {
            What::Picture | What::Video | What::Shader => None,
            What::Scene | What::Web if renderer() => None,
            What::Scene | What::Web => Some(no_renderer()),
            What::Program => Some(
                "Wallpaper Engine application wallpapers are Windows programs and won't run here.",
            ),
        }
    }
}

fn no_renderer() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        format!(
            "Scenes and web wallpapers are drawn by linux-wallpaperengine: {}, and keep Wallpaper Engine in Steam for its assets. Its videos work without it.",
            apolline_wall_protocol::install::hint(apolline_wall_protocol::install::Need::SceneRenderer)
        )
    })
}

/// Whether the program that draws Wallpaper Engine's scenes is installed.
fn renderer() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        std::env::var_os("APOLLINE_WALL_SCENE_RENDERER").is_some()
            || std::env::var_os("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|dir| dir.join("linux-wallpaperengine").is_file())
            })
    })
}

impl From<Kind> for What {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Picture => What::Picture,
            Kind::Video => What::Video,
            Kind::Shader => What::Shader,
            Kind::Scene => What::Scene,
        }
    }
}

const DEPTH: usize = 3;
const THUMB: (u32, u32) = (640, 400);

/// Your folders, then Wallpaper Engine's, newest first.
pub async fn scan(folders: Vec<PathBuf>, engine: Vec<PathBuf>) -> Vec<Item> {
    tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        for folder in &folders {
            walk(folder, DEPTH, &mut items);
        }
        for folder in &engine {
            items.extend(engine::wallpapers(folder));
        }
        items.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
        items.dedup_by(|a, b| a.path == b.path);
        items
    })
    .await
    .unwrap_or_default()
}

fn walk(dir: &Path, depth: usize, items: &mut Vec<Item>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if depth > 0 && !hidden(&path) {
                walk(&path, depth - 1, items);
            }
        } else if let Some(kind) = source::known(&path) {
            items.push(Item {
                name: name_of(&path),
                what: kind.into(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                preview: None,
                workshop: None,
                engine: false,
                path,
            });
        }
    }
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// A file name read as a title: no extension, separators as spaces.
pub fn name_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .replace(['_', '-'], " ")
}

/// A small still of the item, cached by path and modification time. Shaders only have one
/// once the daemon has shown them.
pub async fn thumbnail(item: Item) -> (PathBuf, Option<Handle>) {
    let path = item.path.clone();
    let handle = tokio::task::spawn_blocking(move || make(&item))
        .await
        .ok()
        .flatten();
    (path, handle)
}

fn make(item: &Item) -> Option<Handle> {
    let cache = cached(item);
    if cache.exists() {
        return Some(Handle::from_path(cache));
    }
    std::fs::create_dir_all(cache.parent()?).ok()?;
    let made = match (&item.preview, item.what) {
        (Some(preview), _) => shrink(preview, &cache),
        (None, What::Picture) => shrink(&item.path, &cache),
        (None, What::Video) => frame(&item.path, &cache),
        (None, What::Shader) => {
            let poster = source::poster_path(&item.path);
            poster.exists() && shrink(&poster, &cache)
        }
        (None, _) => false,
    };
    made.then(|| Handle::from_path(cache))
}

fn cached(item: &Item) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    item.path.hash(&mut hasher);
    item.modified.hash(&mut hasher);
    if item.what == What::Shader {
        // A shader's poster changes with the accent, so it is cached only for this session.
        source::poster_path(&item.path)
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .hash(&mut hasher);
    }
    config::cache_dir()
        .join("thumbs")
        .join(format!("{:016x}.png", hasher.finish()))
}

fn shrink(path: &Path, cache: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| still(&bytes))
        .is_some_and(|image| save_thumb(&image, cache))
}

pub fn save_thumb(image: &image::DynamicImage, cache: &Path) -> bool {
    image
        .thumbnail(THUMB.0, THUMB.1)
        .into_rgb8()
        .save(cache)
        .is_ok()
}

/// One picture from an image file; for an animated GIF, a frame from the middle, since many
/// Workshop previews open on black.
pub fn still(bytes: &[u8]) -> Option<image::DynamicImage> {
    if bytes.starts_with(b"GIF8") {
        use image::AnimationDecoder;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        let frames: Vec<image::Frame> = decoder.into_frames().take(48).flatten().collect();
        let frame = frames.get(frames.len() / 2)?;
        return Some(image::DynamicImage::ImageRgba8(frame.buffer().clone()));
    }
    image::load_from_memory(bytes).ok()
}

fn frame(path: &Path, cache: &Path) -> bool {
    ["1", "0"].iter().any(|at| {
        Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-ss", at, "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!("scale={}:-2", THUMB.0))
            .arg(cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
            && cache.exists()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_animated_gif_gives_a_frame_from_its_middle() {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame, Rgba, RgbaImage};
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            for shade in [0u8, 0, 200, 200, 200] {
                let picture = RgbaImage::from_pixel(4, 4, Rgba([shade, shade, shade, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        picture,
                        0,
                        0,
                        Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }
        let middle = still(&bytes).unwrap().into_rgba8();
        assert!(
            middle.get_pixel(1, 1)[0] > 150,
            "{:?}",
            middle.get_pixel(1, 1)
        );
        assert!(still(b"not a picture").is_none());
    }

    #[test]
    fn names_read_like_titles() {
        assert_eq!(
            name_of(Path::new("/w/neon_rain-loop.mp4")),
            "neon rain loop"
        );
        assert_eq!(name_of(Path::new("/w/hills.frag")), "hills");
    }

    #[tokio::test]
    async fn a_scan_finds_known_files_newest_first_and_skips_hidden_folders() {
        let root = std::env::temp_dir().join(format!("apolline-scan-{}", std::process::id()));
        let nested = root.join("set/one");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("old.jpg"), b"").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(nested.join("new.frag"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        std::fs::write(root.join(".hidden/secret.png"), b"").unwrap();
        let items = scan(vec![root.clone(), PathBuf::from("/nowhere")], Vec::new()).await;
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["new", "old"]);
        assert_eq!(items[0].what, What::Shader);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
