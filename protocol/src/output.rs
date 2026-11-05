use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

static OUTPUT: OnceLock<Output> = OnceLock::new();

/// Lines that may wait while the parent isn't reading before the stream counts as lost.
const BACKLOG: usize = 1024;

struct Output {
    lines: mpsc::SyncSender<String>,
    /// What didn't fit in the queue, in order, written once the queue is empty.
    overflow: Arc<Mutex<VecDeque<String>>>,
    failed: Arc<AtomicBool>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

fn is_status(line: &str) -> bool {
    line.starts_with("{\"status\"")
}

impl Output {
    fn new(
        mut writer: impl Write + Send + 'static,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (lines, incoming) = mpsc::sync_channel::<String>(32);
        let overflow = Arc::new(Mutex::new(VecDeque::new()));
        let failed = Arc::new(AtomicBool::new(false));
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
        let waiting = Arc::clone(&overflow);
        let state = Arc::clone(&failed);
        let wake = Arc::clone(&notify);
        std::thread::spawn(move || {
            loop {
                let line = match incoming.try_recv() {
                    Ok(line) => line,
                    Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {
                        let next = waiting.lock().ok().and_then(|mut lines| lines.pop_front());
                        match next {
                            Some(line) => line,
                            // A short wait rather than a blocking one: a line can land in the
                            // overflow just as it empties.
                            None => match incoming.recv_timeout(Duration::from_millis(100)) {
                                Ok(line) => line,
                                Err(RecvTimeoutError::Timeout) => continue,
                                Err(RecvTimeoutError::Disconnected) => break,
                            },
                        }
                    }
                };
                if state.load(Ordering::Acquire) {
                    break;
                }
                if writer
                    .write_all(line.as_bytes())
                    .and_then(|_| writer.write_all(b"\n"))
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    if !state.swap(true, Ordering::AcqRel) {
                        wake();
                    }
                    break;
                }
            }
        });
        Self {
            lines,
            overflow,
            failed,
            notify,
        }
    }

    /// Never blocks the render loop. A parent that reads slowly gets every event in order,
    /// with statuses, which are snapshots, folded into the newest while they wait; one that
    /// stops reading for good loses the stream once the backlog is full.
    fn send(&self, line: &str) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        let fail = || {
            if !self.failed.swap(true, Ordering::AcqRel) {
                (self.notify)();
            }
        };
        if line.len() > 2 * 1024 * 1024 {
            return fail();
        }
        let Ok(mut overflow) = self.overflow.lock() else {
            return fail();
        };
        // Once anything waits, everything after it waits too, so the order holds.
        let line = if overflow.is_empty() {
            match self.lines.try_send(line.to_owned()) {
                Ok(()) => return,
                Err(TrySendError::Full(line)) => line,
                Err(TrySendError::Disconnected(_)) => return fail(),
            }
        } else {
            line.to_owned()
        };
        if is_status(&line) {
            overflow.retain(|waiting| !is_status(waiting));
        }
        overflow.push_back(line);
        if overflow.len() > BACKLOG {
            drop(overflow);
            fail();
        }
    }
}

/// Child mode's events go to stdout from a thread of their own. Losing the stream ends the
/// child session so the parent can restart it and ask for the status again.
pub fn start(notify: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
    if OUTPUT.get().is_some() {
        return Err("stdout event stream already started".into());
    }
    OUTPUT
        .set(Output::new(std::io::stdout(), notify))
        .map_err(|_| "stdout event stream already started".into())
}

pub fn failed() -> bool {
    OUTPUT
        .get()
        .is_some_and(|output| output.failed.load(Ordering::Acquire))
}

pub(crate) fn send(line: &str) {
    if let Some(output) = OUTPUT.get() {
        output.send(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that holds its first write until released, and records everything.
    struct Stalled {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for Stalled {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                let _ = self.release.recv();
            }
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    type Stall = (
        Output,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        Arc<Mutex<Vec<u8>>>,
    );

    fn stalled() -> Stall {
        let (entered, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let output = Output::new(
            Stalled {
                entered: Some(entered),
                release: released,
                written: Arc::clone(&written),
            },
            || {},
        );
        (output, waiting, release, written)
    }

    #[test]
    fn a_slow_parent_gets_every_event_in_order_and_only_the_newest_status() {
        let (output, waiting, release, written) = stalled();
        output.send("first");
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        for n in 0..100 {
            output.send(&format!("event {n}"));
            output.send(&format!("{{\"status\":{n}}}"));
        }
        assert!(!output.failed.load(Ordering::Acquire));
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&written.lock().unwrap()).contains("{\"status\":99}") {
            assert!(
                std::time::Instant::now() < deadline,
                "the backlog never drained"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = String::from_utf8_lossy(&written.lock().unwrap()).into_owned();
        let events: Vec<usize> = text
            .lines()
            .filter_map(|line| line.strip_prefix("event "))
            .map(|n| n.parse().unwrap())
            .collect();
        assert_eq!(events, (0..100).collect::<Vec<_>>());
        assert_eq!(text.lines().last(), Some("{\"status\":99}"));
    }

    #[test]
    fn a_parent_that_stops_reading_loses_the_stream_without_blocking_anyone() {
        let (output, waiting, release, _) = stalled();
        output.send("first");
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..BACKLOG + 64 {
            output.send("event");
        }
        assert!(output.failed.load(Ordering::Acquire));
        release.send(()).unwrap();
    }

    #[test]
    fn a_broken_output_notifies_the_owner() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (notify, notified) = mpsc::channel();
        let output = Output::new(Broken, move || {
            let _ = notify.send(());
        });
        output.send("event");
        notified.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(output.failed.load(Ordering::Acquire));
    }
}
