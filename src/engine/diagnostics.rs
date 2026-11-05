use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::fd::AsRawFd;

const FRAGMENT: usize = 4096;

#[derive(Default)]
struct Tail {
    lines: VecDeque<String>,
    pending: Vec<u8>,
}

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.pending.push(byte);
            if byte == b'\n' || self.pending.len() == FRAGMENT {
                self.finish_line();
            }
        }
    }

    fn finish_line(&mut self) {
        let line = String::from_utf8_lossy(&self.pending).trim().to_owned();
        self.pending.clear();
        if !line.is_empty() {
            self.lines.push_back(line);
            if self.lines.len() > 3 {
                self.lines.pop_front();
            }
        }
    }

    fn finish(mut self) -> String {
        self.finish_line();
        self.lines.into_iter().collect::<Vec<_>>().join(" / ")
    }
}

/// A descendant may keep stderr open after the renderer exits. Its pidfd, rather than
/// pipe EOF, ends collection; a noisy pipe gets bounded work before checking exit again.
pub(super) fn collect(
    exit: &impl AsRawFd,
    stderr: &mut (impl Read + AsRawFd),
) -> io::Result<String> {
    let fd = stderr.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fds = [
        libc::pollfd {
            fd: exit.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let mut tail = Tail::default();
    let mut bytes = [0; FRAGMENT];
    loop {
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
            return Err(io::Error::from_raw_os_error(libc::EBADF));
        }
        if fds[1].revents != 0 {
            for _ in 0..16 {
                match stderr.read(&mut bytes) {
                    Ok(0) => {
                        fds[1].fd = -1;
                        break;
                    }
                    Ok(len) => tail.push(&bytes[..len]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        if fds[0].revents != 0 {
            return Ok(tail.finish());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::process::Process;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn diagnostics_keep_three_bounded_fragments_across_reads() {
        let mut tail = Tail::default();
        tail.push(b"old\npart");
        tail.push(b"ial\nnext\nlast");
        assert_eq!(tail.finish(), "partial / next / last");
        let mut tail = Tail::default();
        tail.push(&vec![b'a'; FRAGMENT * 10 + 1]);
        assert!(tail.pending.len() <= FRAGMENT);
        assert_eq!(tail.lines.len(), 3);
        assert!(tail.finish().len() <= FRAGMENT * 3 + 6);
    }

    #[test]
    fn an_open_pipe_cannot_hide_renderer_exit() {
        let mut child = std::process::Command::new("/bin/true").spawn().unwrap();
        let exit = Process::open(child.id()).unwrap();
        let (mut stderr, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"last diagnostic\n").unwrap();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let tail = collect(&exit, &mut stderr).unwrap();
            let status = child.wait().unwrap();
            done.send((tail, status.success())).unwrap();
        });
        let observed = result.recv_timeout(Duration::from_secs(2));
        drop(writer);
        worker.join().unwrap();
        assert_eq!(observed.unwrap(), ("last diagnostic".into(), true));
    }

    #[test]
    fn pipe_eof_does_not_mean_the_renderer_has_exited() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let exit = Process::open(child.id()).unwrap();
        let (mut stderr, writer) = UnixStream::pair().unwrap();
        drop(writer);
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            done.send(collect(&exit, &mut stderr)).unwrap();
        });
        let early = result.recv_timeout(Duration::from_millis(30));
        child.kill().unwrap();
        child.wait().unwrap();
        worker.join().unwrap();
        assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
        assert_eq!(result.recv().unwrap().unwrap(), "");
    }
}
