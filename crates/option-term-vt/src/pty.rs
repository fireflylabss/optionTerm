//! Pseudo-terminal spawn + I/O.
//!
//! Ported from optionTerm 0.1.x. cwd/busy detection lives in
//! `option_term_core::pty`, driven by the fd that `PaneHandle::pty_fd`
//! exposes.

use std::{
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
};

use nix::{
    errno::Errno,
    fcntl::{self, OFlag},
    pty::{self, ForkptyResult, Winsize},
    sys::{signal, wait},
    unistd::{self, Pid},
};

pub struct Pty {
    master: OwnedFd,
}

/// The spawned child; the pane thread keeps it to reap on exit.
pub enum Child {
    Active(Pid),
    Exited(Pid),
}

impl Child {
    pub fn pid(&self) -> Pid {
        match *self {
            Child::Active(pid) | Child::Exited(pid) => pid,
        }
    }

    /// Exit status if the child already finished, without blocking.
    pub fn exited_status(&mut self) -> Option<i32> {
        let Child::Active(pid) = *self else {
            // Already reaped; nothing new to report.
            return None;
        };
        match wait::waitpid(pid, Some(wait::WaitPidFlag::WNOHANG)) {
            Ok(wait::WaitStatus::Exited(_, status)) => {
                *self = Child::Exited(pid);
                Some(status)
            }
            Ok(wait::WaitStatus::Signaled(_, sig, _)) => {
                *self = Child::Exited(pid);
                Some(128 + sig as i32)
            }
            _ => None,
        }
    }

    /// Reap synchronously. Used after the PTY reports EOF/HUP.
    pub fn wait_status(&mut self) -> Option<i32> {
        let Child::Active(pid) = *self else {
            return None;
        };
        match wait::waitpid(pid, None) {
            Ok(wait::WaitStatus::Exited(_, status)) => {
                *self = Child::Exited(pid);
                Some(status)
            }
            Ok(wait::WaitStatus::Signaled(_, sig, _)) => {
                *self = Child::Exited(pid);
                Some(128 + sig as i32)
            }
            _ => None,
        }
    }

    /// Kill only the child we spawned.
    ///
    /// SIGKILL, not SIGHUP: a hung waitpid during teardown was traced to
    /// SIGHUP not guaranteeing termination (see AGENTS.md). Never signal
    /// process groups or the foreground job — those may outlive the pane.
    pub fn kill(&mut self) {
        if let Child::Active(pid) = *self {
            let _ = signal::kill(pid, signal::SIGKILL);
            let _ = self.wait_status();
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Debug)]
pub enum PtyError {
    EndOfStream,
    Other(Errno),
}

impl Pty {
    /// Fork a PTY and exec `argv` (or a login shell when `None`).
    ///
    /// `env` pairs are set in the child on top of the inherited environment;
    /// the caller supplies TERM/TERM_PROGRAM/COLORTERM.
    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        cwd: Option<&Path>,
        argv: Option<&[String]>,
        env: &[(String, String)],
    ) -> std::io::Result<(Self, Child)> {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: cols.saturating_mul(cell_width),
            ws_ypixel: rows.saturating_mul(cell_height),
        };

        match unsafe { pty::forkpty(&winsize, None)? } {
            ForkptyResult::Child => {
                for (key, value) in env {
                    // SAFETY: single-threaded between fork and exec.
                    unsafe { std::env::set_var(key, value) };
                }
                let mut command = match argv.filter(|a| !a.is_empty()) {
                    Some(argv) => {
                        let mut command = Command::new(&argv[0]);
                        command.args(&argv[1..]);
                        command
                    }
                    None => {
                        let shell = match std::env::var_os("SHELL") {
                            Some(shell) if !shell.is_empty() => PathBuf::from(shell),
                            _ => match unistd::User::from_uid(unistd::getuid()) {
                                Ok(Some(user)) => user.shell,
                                _ => PathBuf::from("/bin/sh"),
                            },
                        };
                        let mut command = Command::new(&shell);
                        // A leading `-` in argv[0] is how a shell knows it is
                        // a login shell.
                        let name = shell.file_name().unwrap_or(shell.as_os_str());
                        let mut arg0 = std::ffi::OsString::from("-");
                        arg0.push(name);
                        command.arg0(arg0);
                        command
                    }
                };
                // A stale directory must not stop the shell from launching.
                if let Some(cwd) = cwd.filter(|p| p.is_dir()) {
                    command.current_dir(cwd);
                }
                let _ = command.exec();
                std::process::exit(127);
            }
            ForkptyResult::Parent { child, master } => {
                let raw_flags = fcntl::fcntl(&master, fcntl::F_GETFL)?;
                let flags = OFlag::from_bits_retain(raw_flags) | OFlag::O_NONBLOCK;
                let _ = fcntl::fcntl(&master, fcntl::F_SETFL(flags))?;
                Ok((Self { master }, Child::Active(child)))
            }
        }
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    /// A dup of the master fd for callers that need a stable handle
    /// (`option_term_core::pty` probes `/proc` through it).
    pub fn dup_fd(&self) -> std::io::Result<OwnedFd> {
        unistd::dup(&self.master).map_err(std::io::Error::from)
    }

    /// One non-blocking read. `EndOfStream` covers EOF and EIO (child gone).
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, PtyError> {
        loop {
            return match unistd::read(&self.master, buf) {
                Ok(len) => Ok(len),
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => Ok(0),
                Err(Errno::EIO) => Err(PtyError::EndOfStream),
                Err(err) => Err(PtyError::Other(err)),
            };
        }
    }

    /// Write all of `data`, briefly parking on a full non-blocking master.
    pub fn write_all(&self, data: &[u8]) {
        write_fd(self.master.as_raw_fd(), data);
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: cols.saturating_mul(cell_width),
            ws_ypixel: rows.saturating_mul(cell_height),
        };
        nix::ioctl_write_ptr_bad!(tiocswinsz, nix::libc::TIOCSWINSZ, Winsize);
        let _ = unsafe { tiocswinsz(self.master.as_raw_fd(), &winsize) };
    }
}

pub fn write_fd(fd: RawFd, data: &[u8]) {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::fd::BorrowedFd;
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let mut remaining = data;
    while !remaining.is_empty() {
        match unistd::write(borrowed, remaining) {
            Ok(len) => remaining = &remaining[len..],
            Err(Errno::EINTR) => continue,
            Err(Errno::EAGAIN) => {
                // Non-blocking master is full (large paste): wait briefly for
                // writability instead of silently dropping the rest.
                let mut fds = [PollFd::new(borrowed, PollFlags::POLLOUT)];
                match poll(&mut fds, PollTimeout::from(1000u16)) {
                    Ok(n) if n > 0 => continue,
                    _ => break,
                }
            }
            Err(_) => break,
        }
    }
}
