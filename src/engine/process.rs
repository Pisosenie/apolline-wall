use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Makes a child go when the daemon goes, however it goes.
pub(super) fn dies_with_us(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

pub(super) struct Process {
    pub id: u64,
    pub pid: u32,
    fd: OwnedFd,
}

impl Process {
    /// Called before the child is reaped, so its PID cannot have been reused.
    pub fn open(pid: u32) -> io::Result<Self> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            pid,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            fd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
        })
    }

    pub fn exit_fd(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }

    pub fn signal(&self, signal: i32) -> io::Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl AsRawFd for Process {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.fd.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_reaped_child_handle_cannot_signal_a_reused_pid() {
        let mut child = std::process::Command::new("/bin/true").spawn().unwrap();
        let process = Process::open(child.id()).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            process.signal(libc::SIGTERM).unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
