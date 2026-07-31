//! Kernel helpers for cwd / busy detection via a VTE PTY fd.

use std::{os::fd::RawFd, path::PathBuf};

use nix::unistd::{self, Pid};

/// Working directory of whatever is currently in the foreground.
///
/// OSC 7 only works when the shell emits it, so this reads the truth from the
/// kernel: the terminal's foreground process group, then that process's `cwd`
/// link. It also follows `cd` inside a running program.
pub fn foreground_cwd(master_fd: RawFd) -> Option<PathBuf> {
    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(master_fd) };
    let pgid = unistd::tcgetpgrp(borrowed).ok()?;
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", pgid.as_raw())).ok()?;
    // A deleted directory resolves to something like "/old/path (deleted)".
    cwd.is_dir().then_some(cwd)
}

/// Whether a program other than the shell itself holds the terminal.
pub fn is_busy(master_fd: RawFd, shell_pid: i32) -> bool {
    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(master_fd) };
    let Ok(foreground) = unistd::tcgetpgrp(borrowed) else {
        return false;
    };
    if shell_pid <= 0 {
        return false;
    }
    foreground != Pid::from_raw(shell_pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::{
        fcntl::{self, OFlag},
        pty::{self, ForkptyResult, Winsize},
        sys::{signal, wait},
        unistd,
    };
    use std::{
        os::{
            fd::{AsRawFd, OwnedFd},
            unix::process::CommandExt,
        },
        process::Command,
    };

    struct TestPty {
        master: OwnedFd,
        child: Pid,
    }

    impl Drop for TestPty {
        fn drop(&mut self) {
            let _ = signal::kill(self.child, signal::SIGHUP);
            let _ = wait::waitpid(self.child, None);
        }
    }

    fn spawn_in(dir: &std::path::Path) -> TestPty {
        let winsize = Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 640,
            ws_ypixel: 384,
        };
        match unsafe { pty::forkpty(&winsize, None).expect("forkpty") } {
            ForkptyResult::Child => {
                let _ = Command::new("/bin/sh")
                    .current_dir(dir)
                    .env("TERM", "xterm-256color")
                    .exec();
                std::process::exit(127);
            }
            ForkptyResult::Parent { child, master } => {
                let raw_flags = fcntl::fcntl(&master, fcntl::F_GETFL).expect("F_GETFL");
                let flags = OFlag::from_bits_retain(raw_flags) | OFlag::O_NONBLOCK;
                let _ = fcntl::fcntl(&master, fcntl::F_SETFL(flags));
                TestPty { master, child }
            }
        }
    }

    #[test]
    fn reads_the_foreground_directory_from_the_kernel() {
        let dir = std::env::temp_dir().canonicalize().expect("temp dir");
        let pty = spawn_in(&dir);

        let mut found = None;
        for _ in 0..200 {
            if let Some(cwd) = foreground_cwd(pty.master.as_raw_fd()) {
                found = Some(cwd);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let found = found.expect("no foreground cwd was ever reported");
        assert_eq!(
            found.canonicalize().ok().as_deref(),
            Some(dir.as_path()),
            "reported {found:?}, expected {dir:?}"
        );
        assert!(!is_busy(pty.master.as_raw_fd(), pty.child.as_raw()));
        let _ = unistd::getuid(); // keep unistd linked for feature parity
    }
}
