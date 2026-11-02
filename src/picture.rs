use std::path::Path;

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};

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

#[cfg(test)]
mod tests {
    use super::*;

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
