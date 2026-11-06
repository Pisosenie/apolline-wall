//! Where the pointer is over each screen, for scenes that follow it: their parallax and the
//! effects that read it. Wayland says when it moves over our surface, which is when it's over
//! the desktop; X11 is asked each frame. Elsewhere it stays where it was last seen, or in the
//! middle.

use super::{App, Desktop, Surface};

impl App {
    /// The pointer at `at` over screen `index`, in its own pixels when it is `size` big.
    pub(super) fn pointed(&mut self, index: usize, at: (f64, f64), size: (u32, u32)) {
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        self.screens[index].pointer = [across(at.0, size.0), -across(at.1, size.1)];
        self.pointed_last = Some(self.screens[index].id);
    }

    /// Where the pointer is over screen `index` now, -1 to 1 across it with y up.
    pub(super) fn pointer(&mut self, index: usize) -> [f32; 2] {
        let over = match (&self.desktop, &self.screens[index].surface) {
            (Desktop::X11(x11), Surface::X11(window)) => x11.pointer().and_then(|(x, y)| {
                let (left, top) = window.position();
                let (width, height) = window.size();
                let at = (
                    f64::from(x) - f64::from(left),
                    f64::from(y) - f64::from(top),
                );
                ((0.0..f64::from(width)).contains(&at.0)
                    && (0.0..f64::from(height)).contains(&at.1))
                .then_some((at, (width, height)))
            }),
            _ => None,
        };
        if let Some((at, size)) = over {
            self.pointed(index, at, size);
        }
        self.screens[index].pointer
    }
}

/// The pointer over a screen, -1 to 1 with y up, over the picture placed on it by `uv`.
pub(super) fn through([x, y, dx, dy]: crate::picture::Uv, [px, py]: [f32; 2]) -> [f32; 2] {
    let (u, v) = ((px + 1.0) / 2.0 * x + dx, (1.0 - py) / 2.0 * y + dy);
    [
        (u * 2.0 - 1.0).clamp(-1.0, 1.0),
        (1.0 - v * 2.0).clamp(-1.0, 1.0),
    ]
}

/// A place along a side `side` pixels long, as -1 to 1.
fn across(at: f64, side: u32) -> f32 {
    ((at / f64::from(side)).clamp(0.0, 1.0) * 2.0 - 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::{across, through};

    #[test]
    fn the_pointer_runs_minus_one_to_one_with_y_up() {
        assert_eq!(
            (across(0.0, 100), across(50.0, 100), across(100.0, 100)),
            (-1.0, 0.0, 1.0)
        );
        assert_eq!(across(-5.0, 100), -1.0);
    }

    #[test]
    fn a_portrait_screen_points_into_the_middle_of_a_wide_scene() {
        // Covered by a 16:9 scene, a 9:16 screen shows its middle 81/256.
        let uv = crate::picture::placement(crate::protocol::Fit::Cover, (1920, 1080), (1080, 1920));
        let [x, y] = through(uv, [1.0, 1.0]);
        assert!(
            (x - 81.0 / 256.0).abs() < 1e-3 && (y - 1.0).abs() < 1e-6,
            "{x} {y}"
        );
        assert_eq!(through(crate::picture::WHOLE, [0.5, -0.25]), [0.5, -0.25]);
    }
}
