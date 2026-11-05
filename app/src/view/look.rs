use apolline_wall_protocol::protocol::Look;
use iced::widget::{Row, button, column, row, slider, space, text, toggler};
use iced::{Alignment, Element, Fill};

use crate::library::{Item, What};
use crate::{App, Message, Tweak, style};

/// The wallpaper's look: tone for everything, pace for what moves. While the wallpaper is on
/// the chosen monitor every change shows there at once, as a draft until it's applied.
pub fn editor<'a>(app: &'a App, item: &'a Item) -> Element<'a, Message> {
    let look = app.look;
    let saved = app.saved_look(&item.path);
    let controls = controls(
        look,
        matches!(item.what, What::Video | What::Shader)
            || (item.what == What::Scene
                && app.daemon.as_ref().is_some_and(|daemon| {
                    daemon.outputs.iter().any(|monitor| {
                        monitor.animated_scene
                            && monitor.source.as_ref() == Some(&item.path)
                            && app
                                .target
                                .as_ref()
                                .is_none_or(|target| target == &monitor.name)
                    })
                })),
        Message::Look,
    );
    let mut actions = Row::new().spacing(6);
    if look != saved {
        actions = actions
            .push(small("Apply", Message::ApplyLook))
            .push(small("Revert", Message::RevertLook));
    }
    if look != Look::default() {
        actions = actions.push(small("Reset", Message::ResetLook));
    }
    controls.push(actions).into()
}

pub fn controls(
    look: Look,
    animated: bool,
    on_change: fn(crate::Tweak) -> Message,
) -> iced::widget::Column<'static, Message> {
    let mut controls = column![
        control(
            "Brightness",
            format!("{:+.0}%", look.brightness * 100.0),
            slider(-1.0..=1.0, look.brightness, move |v| on_change(
                Tweak::Brightness(v)
            ))
            .step(0.02_f32)
            .style(style::rail)
            .into(),
        ),
        control(
            "Contrast",
            format!("{:.0}%", look.contrast * 100.0),
            slider(0.0..=2.0, look.contrast, move |v| on_change(
                Tweak::Contrast(v)
            ))
            .step(0.02_f32)
            .style(style::rail)
            .into(),
        ),
        control(
            "Saturation",
            format!("{:.0}%", look.saturation * 100.0),
            slider(0.0..=2.0, look.saturation, move |v| on_change(
                Tweak::Saturation(v)
            ))
            .step(0.02_f32)
            .style(style::rail)
            .into(),
        ),
    ]
    .spacing(12);
    if animated {
        controls = controls.push(control(
            "Speed",
            format!("{:.2}×", look.speed),
            slider(0.25..=3.0, look.speed, move |v| on_change(Tweak::Speed(v)))
                .step(0.05_f32)
                .style(style::rail)
                .into(),
        ));
    }
    controls = controls.push(
        row![
            text("Mirror").size(13),
            space::horizontal(),
            toggler(look.flip)
                .on_toggle(move |on| on_change(Tweak::Flip(on)))
                .size(20)
                .style(style::switch),
        ]
        .align_y(Alignment::Center),
    );
    controls
}

fn control<'a>(
    label: &'a str,
    value: String,
    slider: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        row![
            text(label).size(13),
            space::horizontal(),
            text(value).size(12).color(style::SOFT)
        ],
        slider
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

fn small(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(12))
        .padding([5, 12])
        .style(style::secondary)
        .on_press(message)
        .into()
}
