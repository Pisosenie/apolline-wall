use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use apolline_wall_protocol::source::{self, Kind};
use iced::widget::image::Handle;

use crate::{config, engine};

pub(crate) static THUMBNAIL_JOBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

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
            What::Picture | What::Video | What::Shader | What::Scene => None,
            What::Web if renderer() => None,
            What::Web => Some(no_renderer()),
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
    let cache = cached(&item);
    if cache.is_file() {
        return (path, Some(Handle::from_path(cache)));
    }
    let Ok(permit) = THUMBNAIL_JOBS.acquire().await else {
        return (path, None);
    };
    let handle = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        make(&item)
    })
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

pub fn preview_file(item: &Item) -> Option<PathBuf> {
    let poster = source::poster_path(&item.path);
    if poster.is_file() {
        return Some(poster);
    }
    make(item)?;
    Some(cached(item))
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

fn picture_file(path: &Path, limit: u64) -> Result<std::io::Take<std::fs::File>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = path.canonicalize().map_err(|err| err.to_string())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|err| err.to_string())?;
    let metadata = file.metadata().map_err(|err| err.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "preview requires a regular image file no larger than {} MiB",
            limit / (1024 * 1024)
        ));
    }
    Ok(file.take(limit + 1))
}

pub fn picture_reader(
    path: &Path,
) -> Result<image::ImageReader<std::io::BufReader<std::io::Take<std::fs::File>>>, String> {
    let file = picture_file(path, 256 * 1024 * 1024)?;
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader)
}

fn shrink(path: &Path, cache: &Path) -> bool {
    let mut bytes = Vec::new();
    picture_file(path, 32 * 1024 * 1024)
        .and_then(|mut file| file.read_to_end(&mut bytes).map_err(|err| err.to_string()))
        .ok()
        .and_then(|_| still(&bytes))
        .is_some_and(|image| save_thumb(&image, cache))
}

struct PendingThumb(PathBuf);

impl PendingThumb {
    fn new(cache: &Path) -> Option<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let stem = cache.file_stem()?.to_str()?;
        let path = cache.with_file_name(format!(
            ".{stem}-{}-{}.png",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        Some(Self(path))
    }

    fn publish(self, cache: &Path) -> bool {
        std::fs::rename(&self.0, cache).is_ok()
    }
}

impl Drop for PendingThumb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn save_thumb(image: &image::DynamicImage, cache: &Path) -> bool {
    let Some(pending) = PendingThumb::new(cache) else {
        return false;
    };
    image
        .thumbnail(THUMB.0, THUMB.1)
        .into_rgb8()
        .save(&pending.0)
        .is_ok()
        && pending.publish(cache)
}

/// One picture from an image file; for an animated GIF, a frame from the middle, since many
/// Workshop previews open on black.
pub fn still(bytes: &[u8]) -> Option<image::DynamicImage> {
    use image::{AnimationDecoder, ImageDecoder};
    if bytes.len() > 32 * 1024 * 1024 {
        return None;
    }
    let limits = || {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        limits
    };
    if bytes.starts_with(b"GIF8") {
        let decode = || {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
            decoder.set_limits(limits()).ok()?;
            Some(decoder)
        };
        // Count without retaining frames, then decode the selected frame. Keeping 48
        // full frames for a thumbnail can consume gigabytes even for a small GIF file.
        let mut count = 0;
        for frame in decode()?.into_frames().take(48) {
            frame.ok()?;
            count += 1;
        }
        if count == 0 {
            return None;
        }
        let frame = decode()?.into_frames().nth(count / 2)?.ok()?;
        return Some(image::DynamicImage::ImageRgba8(frame.into_buffer()));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits());
    reader.decode().ok()
}

fn frame(path: &Path, cache: &Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let Some(pending) = PendingThumb::new(cache) else {
        return false;
    };
    let made = ["1", "0"].iter().any(|at| {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        Command::new("ffmpeg")
            // A Workshop "video" can be a playlist pointing anywhere: local files only.
            .args([
                "-loglevel",
                "error",
                "-y",
                "-protocol_whitelist",
                "file",
                "-ss",
                at,
            ])
            .args(["-threads", "1", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-vf",
            ])
            .arg(format!(
                "scale={}:{}:force_original_aspect_ratio=decrease",
                THUMB.0, THUMB.1
            ))
            .arg(&pending.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .and_then(|mut child| wait_thumbnail(&mut child, deadline))
            .is_some_and(|status| status.success())
            && pending
                .0
                .metadata()
                .is_ok_and(|metadata| metadata.len() > 0)
    });
    made && pending.publish(cache)
}

fn wait_thumbnail(
    child: &mut std::process::Child,
    deadline: std::time::Instant,
) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Err(_) => break,
            Ok(None) => {}
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(50)));
    }
    // Reap before releasing the thumbnail slot; cancellation must not accumulate ffmpeg.
    let _ = child.kill();
    let _ = child.wait();
    None
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
    fn thumbnails_reject_oversized_dimensions_before_decoding() {
        let picture = image::RgbaImage::new(16385, 1);
        for format in [image::ImageFormat::Png, image::ImageFormat::Gif] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            picture.write_to(&mut bytes, format).unwrap();
            assert!(still(bytes.get_ref()).is_none(), "accepted {format:?}");
        }
        let path = std::env::temp_dir().join(format!("wall-large-thumb-{}", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(32 * 1024 * 1024 + 1).unwrap();
        assert!(picture_file(&path, 32 * 1024 * 1024).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(picture_file(&std::env::temp_dir(), 32 * 1024 * 1024).is_err());
    }

    #[test]
    fn a_stalled_video_thumbnail_is_killed_and_reaped() {
        use std::time::{Duration, Instant};
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        assert!(wait_thumbnail(&mut child, Instant::now() + Duration::from_millis(20)).is_none());
        assert!(child.try_wait().unwrap().is_some());
        let mut child = Command::new("true").spawn().unwrap();
        assert!(
            wait_thumbnail(&mut child, Instant::now() + Duration::from_secs(2))
                .unwrap()
                .success()
        );
    }

    #[test]
    fn thumbnail_cache_only_publishes_a_finished_file() {
        let folder = std::env::temp_dir().join(format!("wall-atomic-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let cache = folder.join("thumb.png");
        std::fs::write(&cache, b"old good thumbnail").unwrap();
        let pending = PendingThumb::new(&cache).unwrap();
        let temporary = pending.0.clone();
        std::fs::write(&temporary, b"incomplete").unwrap();
        drop(pending);
        assert!(!temporary.exists());
        assert_eq!(std::fs::read(&cache).unwrap(), b"old good thumbnail");
        let pending = PendingThumb::new(&cache).unwrap();
        std::fs::write(&pending.0, b"new good thumbnail").unwrap();
        assert!(pending.publish(&cache));
        assert_eq!(std::fs::read(&cache).unwrap(), b"new good thumbnail");
        std::fs::remove_dir_all(folder).unwrap();
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
