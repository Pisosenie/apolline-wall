use crate::{App, Message, style};
use apolline_scene::properties::{Kind, Schema, Values};
use iced::widget::{column, pick_list, row, slider, text, text_input, toggler};
use iced::{Element, Task};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Default)]
pub struct Editor {
    pub path: Option<PathBuf>,
    pub schema: Option<Schema>,
    pub values: Values,
    pub error: Option<String>,
    colors: BTreeMap<String, String>,
    generation: u64,
}

#[derive(Clone, Debug)]
pub enum Action {
    Loaded(u64, Result<Schema, String>),
    Set(String, Value),
    Color(String, String),
    Reset,
}

impl App {
    pub fn load_properties(&mut self) -> Task<Message> {
        if self.property_editor.path == self.selected && self.property_editor.schema.is_some() {
            return Task::none();
        }
        self.property_editor.generation = self.property_editor.generation.wrapping_add(1);
        self.property_editor.schema = None;
        self.property_editor.error = None;
        self.property_editor.values.clear();
        self.property_editor.colors.clear();
        let Some(item) = self
            .picked()
            .filter(|item| item.what == crate::library::What::Scene)
        else {
            self.property_editor.path = None;
            return Task::none();
        };
        let path = item.path.clone();
        self.property_editor.values = self
            .config
            .wallpaper_properties
            .get(&path.to_string_lossy().to_string())
            .cloned()
            .unwrap_or_default();
        self.property_editor.path = Some(path.clone());
        let generation = self.property_editor.generation;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    Schema::open(&path).map_err(|err| err.to_string())
                })
                .await
                .map_err(|err| err.to_string())
                .and_then(|result| result)
            },
            move |result| Message::Properties(Action::Loaded(generation, result)),
        )
    }

    pub fn properties(&mut self, action: Action) -> Task<Message> {
        let editor = &mut self.property_editor;
        match action {
            Action::Loaded(generation, result) if generation == editor.generation => match result {
                Ok(schema) => {
                    match schema.resolve(&editor.values) {
                        Ok(_) => {}
                        Err(err) => editor.error = Some(err.to_string()),
                    }
                    editor.schema = Some(schema);
                    editor.sync_colors();
                }
                Err(err) => editor.error = Some(err),
            },
            Action::Loaded(_, _) => {}
            Action::Set(key, value) => {
                editor.store(key, value);
                editor.validate();
            }
            Action::Reset => {
                editor.values.clear();
                editor.error = None;
                editor.sync_colors();
            }
            Action::Color(key, text) => {
                editor.colors.insert(key.clone(), text.clone());
                let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
                let color = if hex.len() == 6 && hex.is_ascii() {
                    (0..3)
                        .map(|at| u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16))
                        .collect::<Result<Vec<_>, _>>()
                        .ok()
                } else {
                    None
                };
                if let Some(channels) = color {
                    editor.store(
                        key,
                        json!(format!(
                            "{} {} {}",
                            f32::from(channels[0]) / 255.0,
                            f32::from(channels[1]) / 255.0,
                            f32::from(channels[2]) / 255.0
                        )),
                    );
                    editor.validate();
                } else {
                    editor.error = Some("Enter a color such as #86bdf0".into());
                }
            }
        }
        Task::none()
    }
}

impl Editor {
    /// Takes a whole set of values, a preset's or pasted ones, keeping only what this
    /// wallpaper has and can hold.
    pub fn take(&mut self, values: &Values) {
        let Some(schema) = &self.schema else {
            return;
        };
        let Ok(resolved) = schema.resolve(values) else {
            return;
        };
        self.values.clear();
        for (key, value) in resolved {
            self.store(key, value);
        }
        self.sync_colors();
        self.validate();
    }

    fn sync_colors(&mut self) {
        self.colors.clear();
        if let Some(schema) = &self.schema {
            for property in &schema.properties {
                if property.kind == Kind::Color
                    && let Some(hex) =
                        color_hex(self.values.get(&property.key).unwrap_or(&property.default))
                {
                    self.colors.insert(property.key.clone(), hex);
                }
            }
        }
    }

    pub fn can_apply(&self) -> bool {
        self.error.is_none() || (self.schema.is_none() && self.values.is_empty())
    }

    fn store(&mut self, key: String, value: Value) {
        if self.schema.as_ref().is_some_and(|schema| {
            schema
                .properties
                .iter()
                .any(|property| property.key == key && property.default == value)
        }) {
            self.values.remove(&key);
        } else {
            self.values.insert(key, value);
        }
    }

    fn validate(&mut self) {
        if self.colors.values().any(|color| {
            let hex = color.trim().strip_prefix('#').unwrap_or(color.trim());
            hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            self.error = Some("Enter a color such as #86bdf0".into());
            return;
        }
        self.error = self
            .schema
            .as_ref()
            .and_then(|schema| schema.resolve(&self.values).err())
            .map(|err| err.to_string());
    }
}

fn color_hex(value: &Value) -> Option<String> {
    let channels = value
        .as_str()?
        .split_whitespace()
        .take(4)
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if channels.len() != 3
        || !channels
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return None;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        (channels[0] * 255.0).round() as u8,
        (channels[1] * 255.0).round() as u8,
        (channels[2] * 255.0).round() as u8
    ))
}

pub fn controls(app: &App) -> Element<'_, Message> {
    let editor = &app.property_editor;
    let mut controls = column![
        text("Wallpaper properties").size(16).font(style::medium()),
        text("Available for supported 2D scenes. Changes apply with Set as wallpaper.")
            .size(12)
            .color(style::SOFT)
    ]
    .spacing(10);
    if let Some(error) = &editor.error {
        controls = controls.push(text(error).size(12).color(style::DANGER));
    }
    if let Some(schema) = &editor.schema {
        for property in &schema.properties {
            let key = property.key.clone();
            let value = editor.values.get(&key).unwrap_or(&property.default);
            let widget: Element<'_, Message> = match &property.kind {
                Kind::Bool => toggler(value.as_bool().unwrap_or(false))
                    .on_toggle(move |on| Message::Properties(Action::Set(key.clone(), json!(on))))
                    .into(),
                Kind::Slider {
                    min,
                    max,
                    fractional,
                } if min.abs() <= f64::from(f32::MAX) && max.abs() <= f64::from(f32::MAX) => {
                    let step = if *fractional {
                        ((*max - *min) / 100.0).max(0.0001) as f32
                    } else {
                        1.0
                    };
                    column![
                        text(format!("{:.2}", value.as_f64().unwrap_or(*min)))
                            .size(12)
                            .color(style::SOFT),
                        slider(
                            *min as f32..=*max as f32,
                            value.as_f64().unwrap_or(*min) as f32,
                            move |v| Message::Properties(Action::Set(key.clone(), json!(v)))
                        )
                        .step(step)
                    ]
                    .spacing(4)
                    .into()
                }
                Kind::Color => text_input(
                    "#86bdf0",
                    editor.colors.get(&key).map(String::as_str).unwrap_or(""),
                )
                .on_input(move |text| Message::Properties(Action::Color(key.clone(), text)))
                .padding(8)
                .style(style::field)
                .into(),
                Kind::Text => text_input("Text", value.as_str().unwrap_or(""))
                    .on_input(move |text| {
                        Message::Properties(Action::Set(key.clone(), json!(text)))
                    })
                    .padding(8)
                    .style(style::field)
                    .into(),
                Kind::Combo(choices) => {
                    let picked = choices
                        .iter()
                        .find(|choice| &choice.value == value)
                        .cloned();
                    pick_list(
                        choices.clone(),
                        picked,
                        move |choice: apolline_scene::properties::Choice| {
                            Message::Properties(Action::Set(key.clone(), choice.value))
                        },
                    )
                    .into()
                }
                _ => text("Not supported yet")
                    .size(12)
                    .color(style::FAINT)
                    .into(),
            };
            controls =
                controls.push(column![row![text(&property.label).size(13)], widget].spacing(6));
        }
    }
    controls
        .push(
            iced::widget::button("Reset properties")
                .style(style::secondary)
                .on_press(Message::Properties(Action::Reset)),
        )
        .push(crate::presets::view(app))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_taken_set_keeps_what_the_wallpaper_has_and_drops_defaults() {
        let schema = Schema::parse(&json!({"general": {"properties": {
            "show": {"type": "bool", "value": true},
            "tint": {"type": "color", "value": "1 1 1"}
        }}}))
        .unwrap();
        let mut editor = Editor {
            schema: Some(schema),
            ..Editor::default()
        };
        editor.take(&Values::from([
            ("show".into(), json!(true)),
            ("tint".into(), json!("1 0 0")),
            ("gone".into(), json!(5)),
        ]));
        assert_eq!(
            editor.values,
            Values::from([("tint".into(), json!("1 0 0"))])
        );
        assert_eq!(
            editor.colors.get("tint").map(String::as_str),
            Some("#ff0000")
        );
        assert!(editor.error.is_none());
    }

    #[test]
    fn changing_another_property_does_not_clear_an_invalid_color() {
        let mut editor = Editor {
            colors: BTreeMap::from([("tint".into(), "#invalid".into())]),
            ..Editor::default()
        };
        editor.store("visible".into(), json!(false));
        editor.validate();
        assert!(editor.error.is_some());
    }
}
