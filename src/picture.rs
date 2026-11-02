use std::path::Path;

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, RgbaImage};

use crate::protocol::Fit;

pub struct Source {
    image: RgbaImage,
    alpha: bool,
}

#[derive(Debug)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode(path: &Path) -> Result<Source, String> {
    let fail = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let mut decoder = ImageReader::open(path)
        .map_err(|err| fail(&err))?
        .with_guessed_format()
        .map_err(|err| fail(&err))?
        .into_decoder()
        .map_err(|err| fail(&err))?;
    let orientation = decoder.orientation().map_err(|err| fail(&err))?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|err| fail(&err))?;
    image.apply_orientation(orientation);
    Ok(Source {
        alpha: image.color().has_alpha(),
        image: image.into_rgba8(),
    })
}

pub fn fit(source: &Source, width: u32, height: u32, fit: Fit) -> Result<Pixels, String> {
    let (sw, sh) = source.image.dimensions();
    let (dw, dh) = match fit {
        Fit::Cover => (width, height),
        Fit::Contain => contained((sw, sh), (width, height)),
    };
    let src = ImageRef::new(sw, sh, source.image.as_raw(), PixelType::U8x4)
        .map_err(|err| err.to_string())?;
    let mut dst = Image::new(dw, dh, PixelType::U8x4);
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
        .use_alpha(source.alpha)
        .fit_into_destination(None);
    Resizer::new()
        .resize(&src, &mut dst, &options)
        .map_err(|err| err.to_string())?;
    let rgba = dst.into_vec();
    Ok(if (dw, dh) == (width, height) {
        Pixels {
            width,
            height,
            rgba,
        }
    } else {
        letterbox(&rgba, (dw, dh), (width, height))
    })
}

fn contained((sw, sh): (u32, u32), (width, height): (u32, u32)) -> (u32, u32) {
    let scale = (width as f64 / sw as f64).min(height as f64 / sh as f64);
    let side = |n: u32, max: u32| ((n as f64 * scale).round() as u32).clamp(1, max);
    (side(sw, width), side(sh, height))
}

fn letterbox(rgba: &[u8], (w, h): (u32, u32), (width, height): (u32, u32)) -> Pixels {
    let mut out = [0, 0, 0, 255].repeat((width * height) as usize);
    let (left, top) = ((width - w) / 2, (height - h) / 2);
    let row = w as usize * 4;
    for y in 0..h as usize {
        let at = ((top as usize + y) * width as usize + left as usize) * 4;
        out[at..at + row].copy_from_slice(&rgba[y * row..(y + 1) * row]);
    }
    Pixels {
        width,
        height,
        rgba: out,
    }
}

pub fn save_png(path: &Path, rgba: &[u8], (w, h): (u32, u32)) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|err| format!("{}: {err}", path.display()))?;
    PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        CompressionType::Fast,
        PngFilter::Adaptive,
    )
    .write_image(rgba, w, h, ExtendedColorType::Rgba8)
    .map_err(|err| format!("{}: {err}", path.display()))
}

const VIDEO: [&str; 11] = [
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "ogv", "ts", "gif",
];

const SHADER: [&str; 2] = ["frag", "glsl"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Picture,
    Video,
    Shader,
}

pub fn kind(path: &Path) -> Kind {
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let is = |list: &[&str]| list.iter().any(|known| known.eq_ignore_ascii_case(ext));
    if is(&VIDEO) {
        Kind::Video
    } else if is(&SHADER) {
        Kind::Shader
    } else {
        Kind::Picture
    }
}

/// A shader draws every pixel every frame, so by default it gets about a 1080p screen's worth.
const SHADER_PIXELS: f64 = 1920.0 * 1080.0;

pub fn shader_size((w, h): (u32, u32), scale: Option<f32>) -> (u32, u32) {
    let scale = scale.map_or_else(
        || (SHADER_PIXELS / (w as f64 * h as f64)).sqrt().min(1.0),
        f64::from,
    );
    let side = |n: u32| ((n as f64 * scale).round() as u32).max(1);
    (side(w), side(h))
}

/// Scale and offset that map the target's 0..1 coordinates onto a source of another aspect.
pub type Uv = [f32; 4];

pub const WHOLE: Uv = [1.0, 1.0, 0.0, 0.0];

/// What GL renders lands bottom row first.
pub const UPSIDE_DOWN: Uv = [1.0, -1.0, 0.0, 1.0];

pub fn placement(fit: Fit, (sw, sh): (u32, u32), (tw, th): (u32, u32)) -> Uv {
    let ratio = (tw as f32 / th as f32) / (sw as f32 / sh as f32);
    let (x, y) = match fit {
        Fit::Cover => (ratio.min(1.0), ratio.recip().min(1.0)),
        Fit::Contain => (ratio.max(1.0), ratio.recip().max(1.0)),
    };
    [x, y, (1.0 - x) / 2.0, (1.0 - y) / 2.0]
}

pub fn canvas_size(
    (vw, vh): (u32, u32),
    targets: impl IntoIterator<Item = (Fit, (u32, u32))>,
) -> (u32, u32) {
    let needed = targets
        .into_iter()
        .map(|(fit, (w, h))| {
            let (x, y) = (w as f64 / vw as f64, h as f64 / vh as f64);
            match fit {
                Fit::Cover => x.max(y),
                Fit::Contain => x.min(y),
            }
        })
        .fold(0.0, f64::max);
    let scale = if needed > 0.0 { needed.min(1.0) } else { 1.0 };
    let side = |n: u32| ((n as f64 * scale).round() as u32).max(1);
    (side(vw), side(vh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_png_reads_back_the_same() {
        let path = std::env::temp_dir().join(format!("apolline-wall-{}.png", std::process::id()));
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        save_png(&path, &rgba, (2, 1)).unwrap();
        let back = image::open(&path).unwrap().into_rgba8();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(back.into_raw(), rgba);
    }

    #[test]
    fn sources_are_told_apart_by_extension() {
        assert_eq!(kind(Path::new("/w/sea.MP4")), Kind::Video);
        assert_eq!(kind(Path::new("/w/loop.gif")), Kind::Video);
        assert_eq!(kind(Path::new("/w/waves.frag")), Kind::Shader);
        assert_eq!(kind(Path::new("/w/fog.jpg")), Kind::Picture);
        assert_eq!(kind(Path::new("/w/noext")), Kind::Picture);
    }

    #[test]
    fn shaders_get_at_most_a_1080p_screen_unless_told_otherwise() {
        assert_eq!(shader_size((1920, 1080), None), (1920, 1080));
        assert_eq!(shader_size((1280, 800), None), (1280, 800));
        assert_eq!(shader_size((3840, 2160), None), (1920, 1080));
        assert_eq!(shader_size((3840, 2160), Some(0.25)), (960, 540));
    }

    #[test]
    fn cover_crops_the_wider_side_and_contain_adds_bars() {
        let wide = (1920, 1080);
        let laptop = (1440, 900);
        let [x, y, dx, dy] = placement(Fit::Cover, wide, laptop);
        assert!((x - 0.9).abs() < 1e-4 && y == 1.0 && (dx - 0.05).abs() < 1e-4 && dy == 0.0);
        let [x, y, dx, dy] = placement(Fit::Contain, wide, laptop);
        assert!(x == 1.0 && (y - 1.0 / 0.9).abs() < 1e-4 && dx == 0.0 && dy < 0.0);
        assert_eq!(placement(Fit::Cover, laptop, laptop), WHOLE);
    }

    #[test]
    fn the_canvas_is_as_big_as_the_largest_monitor_needs_and_never_upscaled() {
        assert_eq!(
            canvas_size((3840, 2160), [(Fit::Cover, (1920, 1080))]),
            (1920, 1080)
        );
        assert_eq!(
            canvas_size(
                (3840, 2160),
                [(Fit::Cover, (1440, 900)), (Fit::Cover, (2560, 1440))]
            ),
            (2560, 1440)
        );
        assert_eq!(
            canvas_size((1280, 720), [(Fit::Cover, (2560, 1440))]),
            (1280, 720)
        );
        assert_eq!(canvas_size((1280, 720), []), (1280, 720));
    }

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Source {
        Source {
            image: RgbaImage::from_pixel(w, h, image::Rgba(rgba)),
            alpha: false,
        }
    }

    fn pixel(p: &Pixels, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * p.width + x) * 4) as usize;
        p.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn cover_fills_the_whole_output() {
        let p = fit(&solid(400, 100, [200, 10, 10, 255]), 160, 90, Fit::Cover).unwrap();
        assert_eq!((p.width, p.height, p.rgba.len()), (160, 90, 160 * 90 * 4));
        assert_eq!(pixel(&p, 0, 0), [200, 10, 10, 255]);
        assert_eq!(pixel(&p, 159, 89), [200, 10, 10, 255]);
    }

    #[test]
    fn contain_centers_the_image_on_black() {
        let p = fit(&solid(100, 100, [10, 200, 10, 255]), 160, 90, Fit::Contain).unwrap();
        assert_eq!((p.width, p.height), (160, 90));
        assert_eq!(pixel(&p, 0, 45), [0, 0, 0, 255]);
        assert_eq!(pixel(&p, 80, 45), [10, 200, 10, 255]);
        assert_eq!(pixel(&p, 159, 45), [0, 0, 0, 255]);
    }

    #[test]
    fn contained_keeps_the_aspect_inside_the_box() {
        assert_eq!(contained((4000, 3000), (1920, 1080)), (1440, 1080));
        assert_eq!(contained((1000, 4000), (1920, 1080)), (270, 1080));
        assert_eq!(contained((1920, 1080), (1920, 1080)), (1920, 1080));
    }

    #[test]
    fn a_missing_file_names_itself() {
        let err = decode(Path::new("/nowhere/wall.jpg")).err().unwrap();
        assert!(err.starts_with("/nowhere/wall.jpg: "), "{err}");
    }
}
