use crate::{App, Message, style};
use apolline_wall_protocol::protocol::Monitor;
use iced::widget::{Row, button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Fill};

pub fn bar(app: &App) -> Element<'_, Message> {
    let Some(daemon) = &app.daemon else {
        return container(
            row![
                container(space())
                    .width(8)
                    .height(8)
                    .style(style::pill(style::DANGER)),
                text("apolline-wall isn't running, so nothing can be set")
                    .size(13)
                    .color(style::SOFT),
                space::horizontal(),
                button(text("Start it").size(13))
                    .padding([6, 14])
                    .style(style::primary)
                    .on_press(Message::Start),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .style(style::panel)
        .into();
    };
    let everything_paused =
        !daemon.outputs.is_empty() && daemon.outputs.iter().all(|monitor| monitor.manual_paused);
    let mut monitors = Row::new().spacing(10).align_y(Alignment::Center);
    for monitor in &daemon.outputs {
        monitors = monitors.push(monitor_card(app, monitor));
    }
    if daemon.outputs.is_empty() {
        monitors = monitors.push(text("Waiting for monitors").size(13).color(style::FAINT));
    }
    let cards = scrollable(monitors)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new(),
        ))
        .width(Fill);
    let pause = button(
        text(if everything_paused {
            "Resume all"
        } else {
            "Pause all"
        })
        .size(13),
    )
    .padding([6, 14])
    .style(style::secondary)
    .on_press_maybe(
        (!daemon.outputs.is_empty()).then_some(Message::Pause(None, !everything_paused)),
    );
    container(row![cards, pause].spacing(12).align_y(Alignment::Center))
        .padding([8, 10])
        .style(style::panel)
        .into()
}

fn monitor_card<'a>(app: &'a App, monitor: &'a Monitor) -> Element<'a, Message> {
    let pause = button(
        text(if monitor.manual_paused {
            "Resume"
        } else {
            "Pause"
        })
        .size(11),
    )
    .padding([3, 9])
    .style(style::secondary)
    .on_press(Message::Pause(
        Some(monitor.name.clone()),
        !monitor.manual_paused,
    ));
    let title = monitor
        .source
        .as_deref()
        .map_or("nothing".to_owned(), |path| app.title(path));
    let showing: String = title.chars().take(100).collect();
    let mut card = column![
        row![
            text(monitor.name.as_str()).size(12).font(style::semibold()),
            space::horizontal(),
            pause
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text(showing).size(12).color(style::SOFT),
    ]
    .spacing(5);
    if let Some(reason) = &monitor.pause_reason {
        card = card.push(text(reason).size(11).color(style::FAINT));
    }
    if monitor.playlist {
        if let Some(status) = &monitor.playlist_status {
            card = card.push(
                text(status.to_string())
                    .size(11)
                    .color(if status.exhausted() {
                        style::DANGER
                    } else {
                        style::FAINT
                    }),
            );
            if status.exhausted() {
                card = card.push(
                    text("Choose a wallpaper or start the playlist again")
                        .size(11)
                        .color(style::SOFT),
                );
            }
        }
        let can_step = monitor
            .playlist_status
            .as_ref()
            .is_none_or(|status| status.can_step);
        let shared = monitor
            .playlist_status
            .as_ref()
            .is_some_and(|status| status.shared);
        card = card.push(
            row![
                button(text("Previous").size(11))
                    .style(style::secondary)
                    .on_press_maybe(can_step.then(|| Message::Step(monitor.name.clone(), true))),
                button(text("Next").size(11))
                    .style(style::secondary)
                    .on_press_maybe(can_step.then(|| Message::Step(monitor.name.clone(), false))),
                button(
                    text(if shared {
                        "Stop shared playlist"
                    } else {
                        "Stop playlist"
                    })
                    .size(11)
                )
                .style(style::secondary)
                .on_press(Message::StopPlaylist(monitor.name.clone())),
            ]
            .spacing(5),
        );
    }
    container(card)
        .width(340)
        .padding(10)
        .style(style::panel)
        .into()
}
