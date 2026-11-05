use apolline_wall_protocol::power::{BatteryMode, Conditions, Policy};

#[derive(Default)]
pub struct ManualPause {
    everywhere: bool,
    paused: std::collections::HashSet<String>,
    resumed: std::collections::HashSet<String>,
}

impl ManualPause {
    pub fn set(&mut self, output: Option<&str>, paused: bool) {
        match output {
            None => {
                self.everywhere = paused;
                self.paused.clear();
                self.resumed.clear();
            }
            Some(name) if paused => {
                self.paused.insert(name.to_owned());
                self.resumed.remove(name);
            }
            Some(name) => {
                self.paused.remove(name);
                if self.everywhere {
                    self.resumed.insert(name.to_owned());
                }
            }
        }
    }
    pub fn is_paused(&self, name: &str) -> bool {
        if self.everywhere {
            !self.resumed.contains(name)
        } else {
            self.paused.contains(name)
        }
    }
}

#[derive(Default)]
pub struct Power {
    pub policy: Policy,
    pub conditions: Conditions,
}

impl Power {
    pub fn pause_reason(&self) -> Option<&'static str> {
        if self.policy.pause_on_sleep && self.conditions.sleeping {
            Some("System sleeping")
        } else if self.policy.pause_on_lock
            && (self.conditions.locked || self.conditions.shell_locked)
        {
            Some("Screen locked")
        } else if self.policy.battery == BatteryMode::Pause && self.conditions.on_battery {
            Some("Battery saver")
        } else {
            None
        }
    }

    pub fn shader_size(&self, size: (u32, u32), requested: Option<f32>) -> (u32, u32) {
        let base = crate::picture::shader_size(size, requested);
        if self.reduced() {
            let cap = crate::picture::shader_size(size, Some(self.policy.battery_scale));
            (base.0.min(cap.0), base.1.min(cap.1))
        } else {
            base
        }
    }

    pub fn reduced(&self) -> bool {
        self.conditions.on_battery && self.policy.battery == BatteryMode::Reduce
    }
    pub fn fps(&self, requested: u32) -> u32 {
        if self.reduced() {
            requested.min(self.policy.battery_fps)
        } else {
            requested
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn battery_resolution_respects_the_default_pixel_budget() {
        let mut power = Power::default();
        power.policy.battery = BatteryMode::Reduce;
        power.conditions.on_battery = true;
        assert_eq!(power.shader_size((7680, 4320), None), (1920, 1080));
        assert_eq!(power.shader_size((1920, 1080), Some(0.25)), (480, 270));
        assert_eq!(power.shader_size((1920, 1080), None), (960, 540));
        power.conditions.on_battery = false;
        assert_eq!(power.shader_size((1920, 1080), None), (1920, 1080));
    }

    #[test]
    fn resume_one_monitor_does_not_resume_the_others() {
        let mut manual = ManualPause::default();
        manual.set(None, true);
        manual.set(Some("DP-1"), false);
        assert!(!manual.is_paused("DP-1"));
        assert!(manual.is_paused("HDMI-A-1"));
        manual.set(Some("DP-1"), true);
        assert!(manual.is_paused("DP-1"));
        manual.set(None, false);
        assert!(!manual.is_paused("HDMI-A-1"));
    }

    #[test]
    fn removing_automatic_reasons_preserves_a_manual_pause() {
        let mut manual = ManualPause::default();
        let mut power = Power::default();
        manual.set(Some("DP-1"), true);
        power.conditions.sleeping = true;
        power.conditions.shell_locked = true;
        power.conditions.sleeping = false;
        assert!(power.pause_reason().is_some());
        power.conditions.shell_locked = false;
        assert!(power.pause_reason().is_none());
        assert!(manual.is_paused("DP-1"));
    }

    #[test]
    fn automatic_reasons_remain_independent() {
        let mut power = Power::default();
        power.conditions.locked = true;
        power.conditions.sleeping = true;
        assert_eq!(power.pause_reason(), Some("System sleeping"));
        power.conditions.sleeping = false;
        assert_eq!(power.pause_reason(), Some("Screen locked"));
        power.conditions.locked = false;
        assert_eq!(power.pause_reason(), None);
    }
    #[test]
    fn battery_caps_never_raise_the_requested_quality() {
        let mut power = Power::default();
        power.policy.battery = BatteryMode::Reduce;
        power.conditions.on_battery = true;
        assert_eq!(power.fps(5), 5);
        assert_eq!(power.fps(60), 15);
        power.conditions.on_battery = false;
        assert_eq!(power.fps(60), 60);
    }
}
