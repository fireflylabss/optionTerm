//! Pseudo-terminal spawn + I/O.

use std::{
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::process::CommandExt,
    },
    path::PathBuf,
    process::Command,
};

use libghostty_vt::Terminal;
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

pub enum Child {
    Active(Pid),
    Exited(Pid),
}

#[derive(Debug)]
pub enum PtyError {
    EndOfStream,
    Other(Errno),
}

impl Pty {
    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        cwd: Option<&std::path::Path>,
    ) -> std::io::Result<(Self, Child)> {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: cols.saturating_mul(cell_width),
            ws_ypixel: rows.saturating_mul(cell_height),
        };

        match unsafe { pty::forkpty(&winsize, None)? } {
            ForkptyResult::Child => {
                let shell = match std::env::var_os("SHELL") {
                    Some(shell) if !shell.is_empty() => PathBuf::from(shell),
                    _ => match unistd::User::from_uid(unistd::getuid()) {
                        Ok(Some(user)) => user.shell,
                        _ => PathBuf::from("/bin/sh"),
                    },
                };
                let arg0 = shell.file_name().unwrap_or(shell.as_os_str());
                let mut command = Command::new(&shell);
                // TERM_PROGRAM / VERSION are how CLIs (Grok, OpenCode, etc.)
                // recognise the host terminal. XTVERSION alone is not enough.
                command
                    .env("TERM", "xterm-ghostty")
                    .env("COLORTERM", "truecolor")
                    .env("TERM_PROGRAM", "optionTerm")
                    .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"))
                    .arg0(arg0);
                // Restored sessions start where they left off; a stale
                // directory must not stop the shell from launching.
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

    pub fn read_into(&self, term: &mut Terminal<'_, '_>) -> Result<(), PtyError> {
        // Cap the bytes consumed per wakeup so a flooding child (e.g. `yes`)
        // cannot starve the UI loop; leftover data re-triggers the fd source.
        const MAX_PER_DISPATCH: usize = 256 * 1024;
        let mut buf = [0u8; 8192];
        let mut consumed = 0usize;
        loop {
            match nix::unistd::read(&self.master, &mut buf) {
                Ok(0) => return Err(PtyError::EndOfStream),
                Ok(len) => {
                    term.vt_write(&buf[..len]);
                    consumed += len;
                    if consumed >= MAX_PER_DISPATCH {
                        return Ok(());
                    }
                }
                Err(Errno::EAGAIN) => return Ok(()),
                Err(Errno::EINTR) => continue,
                Err(Errno::EIO) => return Err(PtyError::EndOfStream),
                Err(err) => return Err(PtyError::Other(err)),
            }
        }
    }

    pub fn write_all(&self, data: &[u8]) {
        write_fd(self.master.as_raw_fd(), data);
    }

    /// Working directory of whatever is currently in the foreground.
    ///
    /// OSC 7 only works when the shell is set up to emit it, which most are
    /// not, so this reads the truth from the kernel instead: the terminal's
    /// foreground process group, then that process's `cwd` link. It also
    /// follows `cd` inside a running program, which OSC 7 cannot.
    pub fn foreground_cwd(&self) -> Option<PathBuf> {
        let pgid = unistd::tcgetpgrp(&self.master).ok()?;
        let cwd = std::fs::read_link(format!("/proc/{}/cwd", pgid.as_raw())).ok()?;
        // A deleted directory resolves to something like "/old/path (deleted)",
        // which would make the new pane fail to chdir.
        cwd.is_dir().then_some(cwd)
    }

    /// Whether a program other than the shell itself holds the terminal.
    ///
    /// The shell keeps its own process group in the foreground while it waits
    /// at a prompt, so a foreground group that is not the child we spawned means
    /// something is actually running.
    pub fn is_busy(&self, child: &Child) -> bool {
        let Ok(foreground) = unistd::tcgetpgrp(&self.master) else {
            return false;
        };
        let shell = match child {
            Child::Active(pid) => *pid,
            // A dead shell is not busy, whatever the terminal still reports.
            Child::Exited(_) => return false,
        };
        foreground != shell
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
        match nix::unistd::write(borrowed, remaining) {
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

impl Drop for Child {
    fn drop(&mut self) {
        match *self {
            Child::Active(pid) | Child::Exited(pid) => {
                // Never block the UI on a stubborn child: SIGHUP, give it a
                // moment, then SIGKILL as a last resort.
                let _ = signal::kill(pid, signal::SIGHUP);
                for _ in 0..20 {
                    match wait::waitpid(pid, Some(wait::WaitPidFlag::WNOHANG)) {
                        Ok(wait::WaitStatus::StillAlive) => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        _ => return,
                    }
                }
                let _ = signal::kill(pid, signal::SIGKILL);
                let _ = wait::waitpid(pid, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cwd fix must work without any shell cooperation: no OSC 7, no shell
    /// integration, just the kernel's view of the foreground process group.
    #[test]
    fn reads_the_foreground_directory_from_the_kernel() {
        let dir = std::env::temp_dir().canonicalize().expect("temp dir");
        // SAFETY: single-threaded setup before the fork; the child execs at once.
        unsafe { std::env::set_var("SHELL", "/bin/sh") };

        let (pty, child) = Pty::spawn(80, 24, 8, 16, Some(&dir)).expect("spawn");

        // The shell has to be scheduled and claim the terminal first.
        let mut found = None;
        for _ in 0..200 {
            if let Some(cwd) = pty.foreground_cwd() {
                found = Some(cwd);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(child);

        let found = found.expect("no foreground cwd was ever reported");
        assert_eq!(
            found.canonicalize().ok().as_deref(),
            Some(dir.as_path()),
            "reported {found:?}, expected {dir:?}"
        );
    }
}
