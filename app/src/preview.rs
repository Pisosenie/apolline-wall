use std::path::PathBuf;
use std::sync::Arc;

use apolline_wall_protocol::protocol::{Fit, Look};
use iced::Task;
use iced::widget::image::Handle;
use image::{DynamicImage, ImageDecoder, RgbaImage};

use crate::library::{self, Item, What};
use crate::{App, Message};

pub struct Preview {
    pub generation: u64,
    pub path: PathBuf,
    pub title: String,
    pub poster: bool,
    pub tunable: bool,
    pub look: Look,
    pub fit: Fit,
    pub handle: Option<Handle>,
    pub error: Option<String>,
    pub preset_name: String,
    base: Option<Arc<RgbaImage>>,
    busy: bool,
}

#[derive(Clone, Debug)]
pub enum Action {
    Open,
    Close,
    Loaded(u64, Result<Arc<RgbaImage>, String>),
    Rendered(u64, Look, Handle),
    Look(crate::Tweak),
    Fit(Fit),
    Apply,
    PresetName(String),
    SavePreset,
    UsePreset(String),
    DeletePreset,
}

impl App {
    pub fn preview(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::PresetName(name) => {
                if let Some(preview) = &mut self.preview {
                    preview.preset_name = name.chars().take(64).collect();
                }
                Task::none()
            }
            Action::UsePreset(name) => {
                if let (Some(preview), Some(look)) =
                    (&mut self.preview, self.config.look_presets.get(&name))
                {
                    preview.preset_name = name;
                    preview.look = *look;
                }
                self.render_preview()
            }
            Action::SavePreset | Action::DeletePreset => {
                let Some(preview) = &mut self.preview else {
                    return Task::none();
                };
                let name = preview.preset_name.trim().to_owned();
                if name.is_empty() {
                    preview.error = Some("Give the preset a name".into());
                    return Task::none();
                }
                let mut config = self.config.clone();
                if matches!(action, Action::DeletePreset) {
                    config.look_presets.remove(&name);
                } else {
                    if config.look_presets.len() >= 128 && !config.look_presets.contains_key(&name)
                    {
                        preview.error =
                            Some("There are already 128 presets; remove one first".into());
                        return Task::none();
                    }
                    config.look_presets.insert(name, preview.look);
                }
                match crate::config::save(&config) {
                    Ok(()) => {
                        self.config = config;
                        preview.error = None;
                    }
                    Err(err) => preview.error = Some(err),
                }
                Task::none()
            }
            Action::Open => {
                let Some(item) = self.picked().cloned() else {
                    return Task::none();
                };
                self.preview_generation = self.preview_generation.wrapping_add(1);
                let generation = self.preview_generation;
                let tunable = item.what.takes_a_look()
                    || (item.what == What::Scene
                        && self.daemon.as_ref().is_some_and(|daemon| {
                            daemon.outputs.iter().any(|monitor| {
                                monitor.native_scene && monitor.source.as_ref() == Some(&item.path)
                            })
                        }));
                self.preview = Some(Preview {
                    generation,
                    path: item.path.clone(),
                    title: item.name.clone(),
                    poster: item.what != What::Picture,
                    tunable,
                    look: if tunable {
                        self.saved_look(&item.path)
                    } else {
                        Look::default()
                    },
                    fit: self.fit,
                    handle: None,
                    error: None,
                    preset_name: String::new(),
                    base: None,
                    busy: false,
                });
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || load(&item))
                            .await
                            .map_err(|err| format!("preview: {err}"))
                            .and_then(|result| result)
                            .map(Arc::new)
                    },
                    move |result| Message::Preview(Action::Loaded(generation, result)),
                )
            }
            Action::Close => {
                self.preview = None;
                Task::none()
            }
            Action::Loaded(generation, result) => {
                let Some(preview) = self.preview.as_mut().filter(|p| p.generation == generation)
                else {
                    return Task::none();
                };
                match result {
                    Ok(base) => preview.base = Some(base),
                    Err(err) => {
                        preview.busy = false;
                        preview.error = Some(err);
                        return Task::none();
                    }
                }
                self.render_preview()
            }
            Action::Rendered(generation, look, handle) => {
                let Some(preview) = self.preview.as_mut().filter(|p| p.generation == generation)
                else {
                    return Task::none();
                };
                preview.busy = false;
                if preview.look == look {
                    preview.handle = Some(handle);
                    Task::none()
                } else {
                    self.render_preview()
                }
            }
            Action::Look(tweak) => {
                if let Some(preview) = &mut self.preview {
                    match tweak {
                        crate::Tweak::Brightness(v) => preview.look.brightness = v,
                        crate::Tweak::Contrast(v) => preview.look.contrast = v,
                        crate::Tweak::Saturation(v) => preview.look.saturation = v,
                        crate::Tweak::Speed(v) => preview.look.speed = v,
                        crate::Tweak::Flip(v) => preview.look.flip = v,
                    }
                }
                self.render_preview()
            }
            Action::Fit(fit) => {
                if let Some(preview) = &mut self.preview {
                    preview.fit = fit;
                }
                Task::none()
            }
            Action::Apply => {
                let Some(preview) = self.preview.take() else {
                    return Task::none();
                };
                self.selected = Some(preview.path);
                self.look = preview.look;
                self.fit = preview.fit;
                self.update(Message::Apply)
            }
        }
    }

    fn render_preview(&mut self) -> Task<Message> {
        let Some(preview) = &mut self.preview else {
            return Task::none();
        };
        if preview.busy {
            return Task::none();
        }
        let Some(base) = preview.base.clone() else {
            return Task::none();
        };
        preview.busy = true;
        let generation = preview.generation;
        let look = preview.look;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || tone(&base, look))
                    .await
                    .map(|image| Handle::from_rgba(image.width(), image.height(), image.into_raw()))
            },
            move |result| match result {
                Ok(handle) => Message::Preview(Action::Rendered(generation, look, handle)),
                Err(err) => Message::Preview(Action::Loaded(generation, Err(err.to_string()))),
            },
        )
    }
}

fn load(item: &Item) -> Result<RgbaImage, String> {
    let path = if item.what == What::Picture {
        item.path.clone()
    } else {
        library::preview_file(item).ok_or("No still preview is available yet")?
    };
    let reader = library::picture_reader(&path)?;
    let mut decoder = reader.into_decoder().map_err(|err| err.to_string())?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) * 4 > 256 * 1024 * 1024 {
        return Err("preview image exceeds 256 MiB".into());
    }
    let orientation = decoder.orientation().map_err(|err| err.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|err| err.to_string())?;
    image.apply_orientation(orientation);
    Ok(image.thumbnail(1280, 720).into_rgba8())
}

fn tone(base: &RgbaImage, look: Look) -> RgbaImage {
    let mut image = base.clone();
    for pixel in image.pixels_mut() {
        let mut color = [0.0; 3];
        for (out, &value) in color.iter_mut().zip(pixel.0.iter()) {
            *out = (f32::from(value) / 255.0 - 0.5) * look.contrast + 0.5 + look.brightness;
        }
        let grey = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        for (out, value) in pixel.0[..3].iter_mut().zip(color) {
            *out =
                ((grey + (value - grey) * look.saturation).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    // The compositor's wallpaper layer is opaque, just like the daemon's final pass.
    for pixel in image.pixels_mut() {
        pixel.0[3] = 255;
    }
    if look.flip {
        image::imageops::flip_horizontal_in_place(&mut image);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_tone_preserves_original_pixels_and_mirrors_the_result() {
        let base = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(tone(&base, Look::default()), base);
        let changed = tone(
            &base,
            Look {
                flip: true,
                saturation: 0.0,
                ..Look::default()
            },
        );
        assert_eq!(changed.get_pixel(0, 0).0, [18, 18, 18, 255]);
        assert_eq!(base.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
