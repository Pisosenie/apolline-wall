use super::{App, system};
use crate::protocol;

impl App {
    pub(super) fn system(&mut self, update: system::Update) {
        let before = self.power.conditions;
        let mut ack = None;
        match update {
            system::Update::Sleep(value, acknowledge) => {
                self.power.conditions.sleeping = value;
                ack = acknowledge;
            }
            system::Update::Lock(value) => self.power.conditions.locked = value,
            system::Update::Battery(value) => self.power.conditions.on_battery = value,
            // A service that went away no longer says what it said last: wallpapers don't
            // stay paused for a battery or a sleep nobody reports any more.
            system::Update::Available(service, available) => match service {
                system::Service::Sleep => {
                    self.power.conditions.sleep_available = available;
                    if !available {
                        self.power.conditions.sleeping = false;
                    }
                }
                system::Service::Lock => self.power.conditions.lock_available = available,
                system::Service::Battery => {
                    self.power.conditions.battery_available = available;
                    if !available {
                        self.power.conditions.on_battery = false;
                    }
                }
            },
        }
        if self.power.conditions != before {
            self.sync_policy();
            protocol::emit(&self.status());
        }
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
    }

    pub(super) fn sync_policy(&mut self) {
        self.sync_pauses();
        self.sync_externals();
        for index in 0..self.screens.len() {
            if !self.is_paused(&self.screens[index].name) {
                self.rescale_scene(index);
            }
            if self.screens[index].fade.is_some() {
                self.draw(index);
            }
        }
        self.pause_scenes();
        self.schedule_rotations();
    }

    pub(super) fn manual_paused(&self, name: &str) -> bool {
        self.manual.is_paused(name)
    }

    pub(super) fn pause_reason(&self, name: &str) -> Option<&'static str> {
        self.power.pause_reason().or_else(|| {
            (self.power.policy.pause_on_fullscreen
                && self
                    .screens
                    .iter()
                    .any(|screen| screen.name == name && screen.covered))
            .then_some("Fullscreen window")
        })
    }

    pub(super) fn is_paused(&self, name: &str) -> bool {
        self.manual_paused(name) || self.pause_reason(name).is_some()
    }

    pub(super) fn effective_fps(&self) -> u32 {
        self.power.fps(self.fps)
    }

    pub(super) fn shader_size(&self, size: (u32, u32)) -> (u32, u32) {
        self.power.shader_size(size, self.scale)
    }
}
