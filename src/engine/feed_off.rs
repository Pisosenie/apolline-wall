//! Built without the `plasma` feature, for PipeWire older than the plugin's bindings take:
//! the plugin's requests are answered with why, and no monitor is ever fed.

use super::App;
use crate::protocol::{self, Event, FeedStream};

pub(super) enum Pipewire {}

pub(super) enum Feed {}

pub(super) enum Change {}

impl App {
    pub(super) fn feed(&mut self, output: &str, _covered: Option<bool>) {
        protocol::emit(&Event::Error {
            output: Some(output.to_owned()),
            source: None,
            message: "Plasma's wallpaper plugin: this apolline-wall was built without it".into(),
        });
    }

    pub(super) fn feed_changed(&mut self, change: Change) {
        match change {}
    }

    pub(super) fn feed_frame(&mut self, _index: usize) {}

    pub(super) fn place_feed(&mut self, _index: usize, _size: (u32, u32)) {}

    pub(super) fn push_feed(&self, _index: usize) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn feed_stream(&self, _index: usize) -> Option<FeedStream> {
        None
    }
}
