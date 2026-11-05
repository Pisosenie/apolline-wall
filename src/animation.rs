use std::time::{Duration, Instant};

const MAX_DELTA: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct Clock {
    elapsed: f64,
    last: Option<Instant>,
    frame: i32,
}

pub struct Frame {
    pub time: f32,
    pub delta: f32,
    pub number: i32,
}

impl Clock {
    /// A replacement of the same scene keeps its phase without counting preparation time.
    pub fn fork(&self) -> Self {
        Self {
            elapsed: self.elapsed,
            last: None,
            frame: self.frame,
        }
    }

    pub fn wait(&self, now: Instant, fps: u32) -> Duration {
        let interval = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
        self.last.map_or(Duration::ZERO, |last| {
            interval.saturating_sub(now.saturating_duration_since(last))
        })
    }

    pub fn tick(&mut self, now: Instant, speed: f32, running: bool, fps: u32) -> Frame {
        let allowed = MAX_DELTA.max(Duration::from_secs_f64(2.0 / f64::from(fps.max(1))));
        let real = if running {
            self.last.map_or(Duration::ZERO, |last| {
                now.saturating_duration_since(last).min(allowed)
            })
        } else {
            Duration::ZERO
        };
        self.last = running.then_some(now);
        let speed = if speed.is_finite() {
            speed.clamp(0.1, 4.0)
        } else {
            1.0
        };
        let delta = real.as_secs_f64() * f64::from(speed);
        self.elapsed += delta;
        Frame {
            time: self.elapsed as f32,
            delta: delta as f32,
            number: self.frame,
        }
    }

    pub fn rendered(&mut self) {
        self.frame = self.frame.saturating_add(1);
    }

    pub fn pause(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_scene_parameters_keeps_phase_without_a_loading_time_jump() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.tick(now, 1.0, true, 30);
        let before = clock.tick(now + Duration::from_millis(20), 1.0, true, 30);
        clock.rendered();
        let mut replacement = clock.fork();
        let after = replacement.tick(now + Duration::from_secs(10), 1.0, true, 30);
        assert_eq!(after.time, before.time);
        assert_eq!(after.delta, 0.0);
        assert_eq!(after.number, before.number + 1);
        assert!(
            (replacement
                .tick(now + Duration::from_millis(10020), 1.0, true, 30)
                .time
                - 0.04)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn a_low_fps_limit_does_not_slow_the_animation_clock() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.tick(now, 1.0, true, 1);
        let frame = clock.tick(now + Duration::from_millis(1050), 1.0, true, 1);
        assert!((frame.time - 1.05).abs() < 1e-6);
        assert!((frame.delta - 1.05).abs() < 1e-6);
    }

    #[test]
    fn speed_applies_to_both_time_and_delta() {
        let now = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.tick(now, 2.0, true, 30).delta, 0.0);
        clock.rendered();
        let frame = clock.tick(now + Duration::from_millis(20), 2.0, true, 30);
        assert!((frame.time - 0.04).abs() < 1e-6);
        assert!((frame.delta - frame.time).abs() < 1e-6);
        assert_eq!(frame.number, 1);
    }

    #[test]
    fn pause_and_manual_redraw_do_not_advance_animation() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.tick(now, 1.0, true, 30);
        let before = clock
            .tick(now + Duration::from_millis(20), 1.0, true, 30)
            .time;
        clock.pause();
        let later = now + Duration::from_secs(3600);
        assert_eq!(clock.tick(later, 1.0, false, 30).time, before);
        assert_eq!(clock.tick(later, 1.0, true, 30).time, before);
        assert_eq!(clock.wait(later, 30), Duration::from_secs_f64(1.0 / 30.0));
    }

    #[test]
    fn a_long_gap_cannot_jump_the_scene_clock() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.tick(now, 1.0, true, 30);
        let frame = clock.tick(now + Duration::from_secs(86400), 2.0, true, 30);
        assert_eq!(frame.time, 0.5);
        assert_eq!(frame.delta, 0.5);
    }

    #[test]
    fn long_running_accumulation_and_frame_overflow_are_safe() {
        let now = Instant::now();
        let mut clock = Clock {
            elapsed: 1_000_000.0,
            last: Some(now),
            frame: i32::MAX,
        };
        clock.tick(now + Duration::from_millis(10), 1.0, true, 30);
        assert!((clock.elapsed - 1_000_000.01).abs() < 1e-8);
        clock.rendered();
        assert_eq!(clock.frame, i32::MAX);
        assert_eq!(clock.wait(now, 240), Duration::from_secs_f64(1.0 / 240.0));
        assert!(clock.tick(now, f32::NAN, true, 30).time.is_finite());
    }
}
