use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryMode {
    #[default]
    Keep,
    Reduce,
    Pause,
}

impl std::fmt::Display for BatteryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep running",
            Self::Reduce => "Reduce shader and scene load",
            Self::Pause => "Pause wallpapers",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub pause_on_lock: bool,
    pub pause_on_sleep: bool,
    pub pause_on_fullscreen: bool,
    pub battery: BatteryMode,
    pub battery_fps: u32,
    pub battery_scale: f32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            pause_on_lock: true,
            pause_on_sleep: true,
            pause_on_fullscreen: true,
            battery: BatteryMode::Keep,
            battery_fps: 15,
            battery_scale: 0.5,
        }
    }
}

impl Policy {
    pub fn check(&self) -> Result<(), String> {
        if !(1..=240).contains(&self.battery_fps) || !(0.1..=1.0).contains(&self.battery_scale) {
            return Err("battery limits require 1–240 fps and scale 0.1–1".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conditions {
    pub sleeping: bool,
    pub locked: bool,
    pub shell_locked: bool,
    pub on_battery: bool,
    pub sleep_available: bool,
    pub lock_available: bool,
    pub battery_available: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub policy: Policy,
    pub conditions: Conditions,
    pub effective_fps: u32,
}
