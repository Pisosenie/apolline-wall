use apolline_wall_protocol::protocol::Fit;
use iced::widget::{
    button, column, container, image, pick_list, row, scrollable, space, text, text_input,
};
use iced::{ContentFit, Element, Fill};

use crate::preview::{Action, Preview};
use crate::{App, Message, style};

pub fn panel<'a>(app: &'a App, state: &'a Preview) -> Element<'a, Message> {
    let frame: Element<'a, Message> = match &state.handle {
        Some(handle) => image(handle.clone())
            .width(Fill)
            .height(Fill)
            .content_fit(match state.fit {
                Fit::Cover => ContentFit::Cover,
                // The preview shows the picture whole; the daemon fills the edges.
                Fit::Contain | Fit::Blur => ContentFit::Contain,
            })
            .into(),
        None => container(
            text(state.error.as_deref().unwrap_or("Preparing preview…")).color(style::SOFT),
        )
        .center(Fill)
        .into(),
    };
    let frame = container(frame)
        .width(Fill)
        .height(Fill)
        .style(style::sunken);
    let controls: Element<'a, Message> = if state.tunable {
        super::look::controls(state.look, false, |tweak| {
            Message::Preview(Action::Look(tweak))
        })
        .into()
    } else {
        text("This wallpaper uses its own renderer; image adjustments are unavailable here.")
            .size(12)
            .color(style::SOFT)
            .into()
    };
    let presets: Element<'a, Message> = if state.tunable {
        column![
            text("Saved looks").size(14).font(style::medium()),
            pick_list(
                app.config.look_presets.keys().cloned().collect::<Vec<_>>(),
                None::<String>,
                |name| Message::Preview(Action::UsePreset(name))
            )
            .placeholder("Choose a preset"),
            text_input("Preset name", &state.preset_name)
                .on_input(|name| Message::Preview(Action::PresetName(name)))
                .padding(8)
                .style(style::field),
            row![
                button("Save look")
                    .style(style::secondary)
                    .on_press(Message::Preview(Action::SavePreset)),
                button("Remove").style(style::secondary).on_press_maybe(
                    app.config
                        .look_presets
                        .contains_key(state.preset_name.trim())
                        .then_some(Message::Preview(Action::DeletePreset))
                ),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into()
    } else {
        space().into()
    };
    let settings = scrollable(
        column![
            text("Image adjustments").font(style::medium()).size(16),
            controls,
            presets,
            text(state.error.as_deref().unwrap_or(""))
                .size(12)
                .color(style::SOFT),
            super::segmented(
                [
                    (Fit::Cover, "Fill"),
                    (Fit::Contain, "Fit"),
                    (Fit::Blur, "Blur")
                ],
                state.fit,
                |fit| Message::Preview(Action::Fit(fit))
            ),
            text(if state.poster {
                "Still preview. Animation and playback speed are not shown."
            } else {
                "Changes stay in this preview until you apply them."
            })
            .size(12)
            .color(style::FAINT),
            button(text("Set as wallpaper"))
                .padding([10, 14])
                .style(style::primary)
                .on_press_maybe(
                    (app.daemon.is_some() && state.handle.is_some())
                        .then_some(Message::Preview(Action::Apply))
                ),
        ]
        .spacing(16),
    )
    .width(260);
    container(
        column![
            row![
                text(&state.title).size(20).font(style::semibold()),
                space::horizontal(),
                button("Close")
                    .style(style::secondary)
                    .on_press(Message::Preview(Action::Close))
            ]
            .spacing(12),
            row![frame, settings].spacing(20).height(Fill),
        ]
        .spacing(16),
    )
    .padding(20)
    .max_width(1120)
    .height(Fill)
    .max_height(700)
    .style(style::panel)
    .into()
}
