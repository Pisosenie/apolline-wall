//! Settings that follow a monitor rather than its connector: after a replug or behind a dock
//! DP-1 can come back as DP-2, and its wallpaper, look and playlist come with it.

use super::App;
use crate::protocol::{self, Event};
use crate::state;

impl App {
    /// Notes which monitor `connector` has, and moves to it what that monitor had on the
    /// connector it was last seen on.
    pub(super) fn recognise(&mut self, connector: &str, identity: &str) {
        let connected: Vec<&str> = self
            .screens
            .iter()
            .map(|screen| screen.name.as_str())
            .filter(|name| *name != connector)
            .collect();
        let Some(persist) = &mut self.persist else {
            return;
        };
        let moved = persist.state.moved(connector, identity, &connected);
        if let Some(old) = &moved {
            persist.state.rename(old, connector);
            persist.good.rename(old, connector);
        }
        let changed = persist.state.seen(connector, identity) | moved.is_some();
        persist.good.seen(connector, identity);
        if changed && let Err(message) = state::save(&persist.path, &persist.state) {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
        let Some(old) = moved else {
            return;
        };
        if let Some(mut show) = self.selection.per_output.remove(&old) {
            show.output = Some(connector.to_owned());
            self.selection.per_output.insert(connector.to_owned(), show);
        }
        if self.selection.inherited.remove(&old) {
            self.selection.inherited.insert(connector.to_owned());
        }
        if let Some(mut rotation) = self.rotations.remove(&Some(old)) {
            rotation.moved_to(connector);
            self.rotations.insert(Some(connector.to_owned()), rotation);
            self.schedule_rotations();
        }
    }
}
