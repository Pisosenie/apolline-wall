use crate::{App, Message, style};
use apolline_wall_protocol::power::BatteryMode;
use iced::widget::{column, pick_list, row, space, text, toggler};
use iced::{Alignment, Element};

#[derive(Clone, Debug)]
pub enum Change {
    Lock(bool),
    Sleep(bool),
    Fullscreen(bool),
    Battery(BatteryMode),
}

pub fn controls(app: &App) -> Element<'_, Message> {
    let Some(daemon) = &app.daemon else {
        return text("Start the wallpaper daemon to configure automatic playback rules.")
            .size(13)
            .color(style::SOFT)
            .into();
    };
    let policy = &daemon.power.policy;
    let conditions = daemon.power.conditions;
    column![
        row![text("Pause while the screen is locked").size(14), space::horizontal(), toggler(policy.pause_on_lock).on_toggle(|v| Message::Power(Change::Lock(v))).style(style::switch)].align_y(Alignment::Center),
        row![text("Pause before sleep").size(14), space::horizontal(), toggler(policy.pause_on_sleep).on_toggle(|v| Message::Power(Change::Sleep(v))).style(style::switch)].align_y(Alignment::Center),
        row![text("Pause behind fullscreen windows (X11)").size(14), space::horizontal(), toggler(policy.pause_on_fullscreen).on_toggle(|v| Message::Power(Change::Fullscreen(v))).style(style::switch)].align_y(Alignment::Center),
        row![text("On battery").size(14), space::horizontal(), pick_list([BatteryMode::Keep, BatteryMode::Reduce, BatteryMode::Pause], Some(policy.battery), |v| Message::Power(Change::Battery(v)))].align_y(Alignment::Center),
        text(format!("Reduced shader and scene load: at most {} fps and {}% resolution. Your own lower limits still apply. Video decoding keeps its usual quality; choose Pause to stop it.", policy.battery_fps, (policy.battery_scale * 100.0).round())).size(12).color(style::SOFT),
        text(format!("System integration: sleep {}, lock {}, battery {}.", available(conditions.sleep_available), available(conditions.lock_available), available(conditions.battery_available))).size(12).color(style::FAINT),
        text(if !conditions.battery_available { "Power source unavailable" } else if conditions.on_battery { "Running on battery" } else { "External power connected" }).size(12).color(style::SOFT),
        text("Automatic rules never clear your manual pause.").size(12).color(style::FAINT),
    ].spacing(12).into()
}

fn available(available: bool) -> &'static str {
    if available {
        "connected"
    } else {
        "unavailable"
    }
}
