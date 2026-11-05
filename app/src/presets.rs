//! Named sets of a wallpaper's property values, kept per wallpaper, and values copied to and
//! pasted from the clipboard as JSON, to pass a setup on to someone else.

use apolline_scene::properties::Values;
use iced::widget::{Row, button, column, row, text, text_input};
use iced::{Alignment, Element, Fill, Task};

use crate::{App, Message, config, style};

/// Pasted text larger than this isn't a set of property values.
const PASTE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub enum Preset {
    Name(String),
    Save,
    Use(String),
    Delete(String),
    Copy,
    Paste,
    Pasted(Option<String>),
}

impl App {
    fn wallpaper_key(&self) -> Option<String> {
        self.selected
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
    }

    pub fn preset(&mut self, preset: Preset) -> Task<Message> {
        match preset {
            Preset::Name(name) => {
                self.preset_name = name;
                Task::none()
            }
            Preset::Save => {
                let name = self.preset_name.trim().to_owned();
                let Some(key) = self.wallpaper_key() else {
                    return Task::none();
                };
                if name.is_empty() || name.chars().count() > 64 {
                    self.notice = Some("A preset needs a name of up to 64 characters.".into());
                    return Task::none();
                }
                let values = self.property_editor.values.clone();
                self.config
                    .property_presets
                    .entry(key)
                    .or_default()
                    .insert(name, values);
                self.preset_name.clear();
                self.notice = config::save(&self.config).err();
                Task::none()
            }
            Preset::Use(name) => {
                let chosen = self
                    .wallpaper_key()
                    .and_then(|key| self.config.property_presets.get(&key))
                    .and_then(|presets| presets.get(&name))
                    .cloned();
                if let Some(values) = chosen {
                    self.property_editor.take(&values);
                }
                Task::none()
            }
            Preset::Delete(name) => {
                if let Some(key) = self.wallpaper_key()
                    && let Some(presets) = self.config.property_presets.get_mut(&key)
                {
                    presets.remove(&name);
                    if presets.is_empty() {
                        self.config.property_presets.remove(&key);
                    }
                    self.notice = config::save(&self.config).err();
                }
                Task::none()
            }
            Preset::Copy => {
                let json =
                    serde_json::to_string_pretty(&self.property_editor.values).unwrap_or_default();
                iced::clipboard::write(json)
            }
            Preset::Paste => {
                iced::clipboard::read().map(|text| Message::Preset(Preset::Pasted(text)))
            }
            Preset::Pasted(text) => {
                match text.as_deref().and_then(parse) {
                    Some(values) => self.property_editor.take(&values),
                    None => {
                        self.notice =
                            Some("The clipboard doesn't hold property values copied here.".into())
                    }
                }
                Task::none()
            }
        }
    }
}

/// Property values as copied: a JSON object of names to values.
fn parse(text: &str) -> Option<Values> {
    if text.len() > PASTE_BYTES {
        return None;
    }
    let values: Values = serde_json::from_str(text).ok()?;
    (values.len() <= 512).then_some(values)
}

/// Saving, choosing and sharing sets of values, under the property controls.
pub fn view(app: &App) -> Element<'_, Message> {
    let saved = app.selected.as_ref().and_then(|path| {
        app.config
            .property_presets
            .get(&path.to_string_lossy().into_owned())
    });
    let mut list = column![].spacing(6);
    for name in saved.into_iter().flat_map(|presets| presets.keys()) {
        list = list.push(
            row![
                button(text(name.as_str()).size(12))
                    .padding([5, 10])
                    .width(Fill)
                    .style(style::secondary)
                    .on_press(Message::Preset(Preset::Use(name.clone()))),
                button(text("Delete").size(12))
                    .padding([5, 10])
                    .style(style::secondary)
                    .on_press(Message::Preset(Preset::Delete(name.clone()))),
            ]
            .spacing(6),
        );
    }
    let small = |label: &'static str, preset: Preset| {
        button(text(label).size(12))
            .padding([5, 10])
            .style(style::secondary)
            .on_press(Message::Preset(preset))
    };
    column![
        text("Presets").size(13),
        row![
            text_input("Name these values", &app.preset_name)
                .on_input(|name| Message::Preset(Preset::Name(name)))
                .on_submit(Message::Preset(Preset::Save))
                .padding([6, 10])
                .size(13)
                .style(style::field),
            small("Save", Preset::Save),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        list,
        Row::new()
            .spacing(6)
            .push(small("Copy values", Preset::Copy))
            .push(small("Paste values", Preset::Paste)),
    ]
    .spacing(8)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_values_must_be_a_bounded_json_object() {
        assert_eq!(
            parse(r#"{"tint": "1 0 0", "show": false}"#).map(|values| values.len()),
            Some(2)
        );
        assert!(parse("[1, 2]").is_none());
        assert!(parse("not json").is_none());
        assert!(parse(&format!("{{\"a\": \"{}\"}}", "x".repeat(PASTE_BYTES))).is_none());
    }
}
