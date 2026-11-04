use apolline_wall_protocol::install::{self, Need};
use iced::widget::{Row, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill};

use crate::download::Act;
use crate::steamcmd::{self, Job, Prompt};
use crate::{App, Message, style};

/// The Steam account steamcmd downloads with, for Settings.
pub fn account(app: &App) -> Element<'_, Message> {
    if !steamcmd::installed() {
        return text(format!(
            "Without the Steam client, Workshop downloads go through steamcmd: {}, then sign in here with an account that owns Wallpaper Engine.",
            install::hint(Need::Steamcmd)
        ))
        .size(12)
        .color(style::FAINT)
        .into();
    }
    let busy = app.steam.run.is_some();
    let mut sign_in = button(text("Sign in").size(13))
        .padding([8, 14])
        .style(style::secondary);
    let mut field = text_input("Steam account name", &app.steam.account)
        .padding([8, 12])
        .size(14)
        .style(style::field);
    if !busy {
        sign_in = sign_in.on_press(Message::Steam(Act::SignIn));
        field = field
            .on_input(|name| Message::Steam(Act::Account(name)))
            .on_submit(Message::Steam(Act::SignIn));
    }
    let mut body = column![
        row![field, sign_in].spacing(8),
        text("For Workshop downloads without the Steam client. The account has to own Wallpaper Engine. The password and Steam Guard code go straight to steamcmd, which keeps the sign-in; only the name is saved here.")
            .size(12)
            .color(style::FAINT),
    ]
    .spacing(10);
    if let Some(run) = progress(app) {
        body = body.push(run);
    } else if app.steam.signed_in {
        body = body.push(
            text("Signed in, and the account owns Wallpaper Engine. steamcmd keeps the sign-in for downloads.")
                .size(13)
                .color(style::SOFT),
        );
    }
    body.into()
}

/// What steamcmd is doing now and the question it asks, if any.
pub fn progress(app: &App) -> Option<Element<'_, Message>> {
    let run = app.steam.run.as_ref()?;
    let mut doing = match &run.job {
        Job::SignIn => "Signing in to Steam".to_owned(),
        Job::Download(_) => format!("Downloading “{}”", run.title),
    };
    if run.size > 0 && run.done > 0 {
        doing.push_str(&format!(" · {}%", (run.done * 100 / run.size).min(99)));
    }
    let waiting = app.steam.queue.len();
    if waiting > 0 {
        doing.push_str(&format!(" · {waiting} more waiting"));
    }
    let mut line = Row::new()
        .spacing(10)
        .align_y(Alignment::Center)
        .push(text(doing).size(13).width(Fill));
    line = line.push(
        button(text("Stop").size(12))
            .padding([5, 12])
            .style(style::secondary)
            .on_press(Message::Steam(Act::Cancel)),
    );
    let mut lines = column![line].spacing(10);
    match run.asked {
        Some(Prompt::Approval) => {
            lines = lines.push(
                text("Approve the sign-in in the Steam app on your phone.")
                    .size(13)
                    .color(style::SOFT),
            );
        }
        Some(prompt) => {
            let (hint, secret) = match prompt {
                Prompt::Password => ("Steam password", true),
                _ => ("Steam Guard code", false),
            };
            lines = lines.push(
                row![
                    text_input(hint, &app.steam.answer)
                        .secure(secret)
                        .on_input(|text| Message::Steam(Act::Answer(text)))
                        .on_submit(Message::Steam(Act::Send))
                        .padding([8, 12])
                        .size(14)
                        .style(style::field),
                    button(text("Send").size(13))
                        .padding([8, 14])
                        .style(style::primary)
                        .on_press(Message::Steam(Act::Send)),
                ]
                .spacing(8),
            );
        }
        None => {}
    }
    Some(
        container(lines)
            .padding([10, 12])
            .width(Fill)
            .style(style::sunken)
            .into(),
    )
}
