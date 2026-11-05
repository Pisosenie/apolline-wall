use super::{App, Desktop, Surface, X11};
use crate::protocol;
use std::collections::HashSet;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, MapState,
};

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Rect {
    fn covers(self, other: Self) -> bool {
        self.width > 0
            && self.height > 0
            && other.width > 0
            && other.height > 0
            && self.x <= other.x
            && self.y <= other.y
            && i64::from(self.x) + i64::from(self.width)
                >= i64::from(other.x) + i64::from(other.width)
            && i64::from(self.y) + i64::from(self.height)
                >= i64::from(other.y) + i64::from(other.height)
    }
}

impl X11 {
    fn words(&self, window: u32, atom: u32, kind: AtomEnum, limit: u32) -> Vec<u32> {
        self.conn
            .get_property(false, window, atom, kind, 0, limit)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|words| words.collect()))
            .unwrap_or_default()
    }

    fn fullscreen_rect(
        &self,
        window: u32,
        desktop: Option<u32>,
        active: Option<u32>,
        excluded_pids: &[u32],
    ) -> Option<Rect> {
        let atoms = &self.atoms;
        let states = self.words(window, atoms._NET_WM_STATE, AtomEnum::ATOM, 64);
        if !states.contains(&atoms._NET_WM_STATE_FULLSCREEN)
            || states.contains(&atoms._NET_WM_STATE_HIDDEN)
        {
            return None;
        }
        if self
            .words(window, atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, 32)
            .contains(&atoms._NET_WM_WINDOW_TYPE_DESKTOP)
        {
            return None;
        }
        if self
            .words(window, atoms._NET_WM_PID, AtomEnum::CARDINAL, 1)
            .first()
            .is_some_and(|pid| excluded_pids.contains(pid))
        {
            return None;
        }
        if let Some(current) = desktop {
            match self
                .words(window, atoms._NET_WM_DESKTOP, AtomEnum::CARDINAL, 1)
                .first()
                .copied()
            {
                Some(on) if on != current && on != u32::MAX => return None,
                None if active != Some(window) => return None,
                _ => {}
            }
        }
        if self
            .words(window, atoms._NET_WM_WINDOW_OPACITY, AtomEnum::CARDINAL, 1)
            .first()
            .is_some_and(|opacity| *opacity != u32::MAX)
        {
            return None;
        }
        let attrs = self.conn.get_window_attributes(window).ok()?.reply().ok()?;
        if attrs.map_state != MapState::VIEWABLE {
            return None;
        }
        let geometry = self.conn.get_geometry(window).ok()?.reply().ok()?;
        let origin = self
            .conn
            .translate_coordinates(window, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        Some(Rect {
            x: origin.dst_x.into(),
            y: origin.dst_y.into(),
            width: geometry.width.into(),
            height: geometry.height.into(),
        })
    }

    fn coverage(
        &mut self,
        outputs: &[(u32, String, Rect)],
        excluded_pids: &[u32],
    ) -> HashSet<String> {
        let mut windows = self.words(
            self.root,
            self.atoms._NET_CLIENT_LIST_STACKING,
            AtomEnum::WINDOW,
            4096,
        );
        if windows.is_empty() {
            windows = self.words(
                self.root,
                self.atoms._NET_CLIENT_LIST,
                AtomEnum::WINDOW,
                4096,
            );
        }
        let watched: HashSet<_> = windows
            .iter()
            .copied()
            .filter(|id| !outputs.iter().any(|(own, _, _)| own == id))
            .collect();
        for &window in watched.difference(&self.watched) {
            let _ = self.conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new()
                    .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
            );
        }
        for &window in self.watched.difference(&watched) {
            let _ = self.conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );
        }
        self.watched = watched;
        let desktop = self
            .words(
                self.root,
                self.atoms._NET_CURRENT_DESKTOP,
                AtomEnum::CARDINAL,
                1,
            )
            .first()
            .copied();
        let active = self
            .words(
                self.root,
                self.atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW,
                1,
            )
            .first()
            .copied();
        self.fullscreen = windows
            .iter()
            .copied()
            .filter(|window| {
                self.watched.contains(window)
                    && self
                        .words(*window, self.atoms._NET_WM_STATE, AtomEnum::ATOM, 64)
                        .contains(&self.atoms._NET_WM_STATE_FULLSCREEN)
            })
            .collect();
        let mut covered = HashSet::new();
        for &window in &self.fullscreen {
            if let Some(rect) = self.fullscreen_rect(window, desktop, active, excluded_pids) {
                for (_, name, output) in outputs {
                    if rect.covers(*output) {
                        covered.insert(name.clone());
                    }
                }
            }
        }
        self.flush();
        covered
    }

    pub(super) fn coverage_property(&self, window: u32, atom: u32) -> bool {
        if window == self.root {
            [
                self.atoms._NET_CLIENT_LIST,
                self.atoms._NET_CLIENT_LIST_STACKING,
            ]
            .contains(&atom)
                || (!self.fullscreen.is_empty()
                    && [
                        self.atoms._NET_CURRENT_DESKTOP,
                        self.atoms._NET_ACTIVE_WINDOW,
                    ]
                    .contains(&atom))
        } else if !self.watched.contains(&window) {
            false
        } else if atom == self.atoms._NET_WM_STATE {
            self.fullscreen.contains(&window)
                || self
                    .words(window, atom, AtomEnum::ATOM, 64)
                    .contains(&self.atoms._NET_WM_STATE_FULLSCREEN)
        } else {
            self.fullscreen.contains(&window)
                && [
                    self.atoms._NET_WM_DESKTOP,
                    self.atoms._NET_WM_WINDOW_OPACITY,
                    self.atoms._NET_WM_WINDOW_TYPE,
                    self.atoms._NET_WM_PID,
                ]
                .contains(&atom)
        }
    }
}

impl App {
    pub(in crate::engine) fn x11_coverage(&mut self) {
        let outputs: Vec<_> = self
            .screens
            .iter()
            .filter_map(|screen| match &screen.surface {
                Surface::X11(window) => Some((
                    window.id,
                    screen.name.clone(),
                    Rect {
                        x: window.position.0.into(),
                        y: window.position.1.into(),
                        width: window.size.0,
                        height: window.size.1,
                    },
                )),
                _ => None,
            })
            .collect();
        let mut excluded = self.external_pids();
        excluded.push(std::process::id());
        let Desktop::X11(x11) = &mut self.desktop else {
            return;
        };
        let covered = x11.coverage(&outputs, &excluded);
        let mut changed = false;
        for screen in &mut self.screens {
            let value = covered.contains(&screen.name);
            changed |= screen.covered != value;
            screen.covered = value;
        }
        if changed {
            self.sync_policy();
            protocol::emit(&self.status());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fullscreen_coverage_is_per_monitor_and_handles_negative_positions() {
        let left = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let right = Rect { x: 0, ..left };
        assert!(left.covers(left));
        assert!(!left.covers(right));
        assert!(
            !Rect {
                width: 1919,
                ..left
            }
            .covers(left)
        );
        let both = Rect {
            width: 3840,
            ..left
        };
        assert!(both.covers(left) && both.covers(right));
        assert!(!Rect { height: 0, ..both }.covers(left));
    }
}
