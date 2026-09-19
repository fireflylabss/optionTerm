//! One terminal pane: a thread owning the `!Send` emulator plus its PTY.
//!
//! The thread polls the PTY master and a wake pipe fed by `PaneHandle::send`.
//! Input flows in through the channel, PTY bytes through `Emulator::feed`,
//! and everything the emulator wants to say back goes out as `Event`s.

use std::{
    os::fd::{AsRawFd, OwnedFd, RawFd},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use nix::{
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd,
};

use crate::{
    emulator::{Emulator, EmulatorOptions},
    input::{Event, Input},
    pty::{Child, Pty, PtyError},
};

pub struct PaneOptions {
    pub emulator: EmulatorOptions,
    pub cwd: Option<PathBuf>,
    /// `None` spawns a login shell (`$SHELL`).
    pub argv: Option<Vec<String>>,
    /// Extra env pairs; TERM/TERM_PROGRAM/COLORTERM are always set.
    pub env: Vec<(String, String)>,
    /// Minimum spacing between `Event::Frame`s; use 8 ms for ~120 fps.
    pub min_frame_interval: Duration,
}

/// Handle to a running pane. Cheap to clone; the thread owns the PTY master
/// and the emulator.
#[derive(Clone)]
pub struct PaneHandle {
    input_tx: Sender<Input>,
    /// Writing one byte wakes the poll loop to drain the channel.
    wake: Arc<OwnedFd>,
    child_pid: Option<i32>,
    /// A dup of the PTY master, kept open for `option_term_core::pty`
    /// (`/proc` cwd/busy probes). Read-only usage; the thread owns the fd.
    pty_fd: Arc<OwnedFd>,
}

impl PaneHandle {
    pub fn send(&self, input: Input) {
        if self.input_tx.send(input).is_err() {
            return;
        }
        let _ = unistd::write(&self.wake, &[0u8]);
    }

    pub fn child_pid(&self) -> Option<i32> {
        self.child_pid
    }

    /// Raw fd for `option_term_core::pty` helpers. Valid while any
    /// `PaneHandle` clone is alive; do not read/write it.
    pub fn pty_fd(&self) -> RawFd {
        self.pty_fd.as_raw_fd()
    }
}

pub fn spawn(
    opts: PaneOptions,
    on_event: Box<dyn Fn(Event) + Send + 'static>,
) -> Result<PaneHandle> {
    let (input_tx, input_rx) = mpsc::channel::<Input>();
    // Self-pipe to wake `poll` when an input arrives.
    let (wake_read, wake_write) = unistd::pipe().context("wake pipe")?;
    for fd in [&wake_read, &wake_write] {
        let flags = nix::fcntl::fcntl(fd, nix::fcntl::F_GETFL)?;
        nix::fcntl::fcntl(
            fd,
            nix::fcntl::F_SETFL(
                nix::fcntl::OFlag::from_bits_retain(flags) | nix::fcntl::OFlag::O_NONBLOCK,
            ),
        )?;
    }

    let PaneOptions {
        emulator: emu_opts,
        cwd,
        argv,
        env: extra_env,
        min_frame_interval,
    } = opts;

    let mut env = vec![
        ("TERM".to_string(), term_name()),
        ("TERM_PROGRAM".to_string(), "optionterm".to_string()),
        (
            "TERM_PROGRAM_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        ),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    env.extend(extra_env);

    let (pty, child) = Pty::spawn(
        emu_opts.cols,
        emu_opts.rows,
        emu_opts.cell_w_px,
        emu_opts.cell_h_px,
        cwd.as_deref(),
        argv.as_deref(),
        &env,
    )
    .context("spawn pty")?;
    let pty_fd = pty.dup_fd().context("dup pty fd")?;
    let child_pid = Some(child.pid().as_raw());

    let handle = PaneHandle {
        input_tx,
        wake: Arc::new(wake_write),
        child_pid,
        pty_fd: Arc::new(pty_fd),
    };

    std::thread::Builder::new()
        .name("option-term-vt pane".to_string())
        .spawn(move || {
            run(
                emu_opts,
                min_frame_interval,
                pty,
                child,
                input_rx,
                wake_read,
                on_event,
            )
        })
        .context("spawn pane thread")?;

    Ok(handle)
}

fn run(
    emu_opts: EmulatorOptions,
    min_interval: Duration,
    pty: Pty,
    mut child: Child,
    input_rx: Receiver<Input>,
    wake_read: OwnedFd,
    on_event: Box<dyn Fn(Event) + Send>,
) {
    let mut emulator = match Emulator::new(emu_opts) {
        Ok(e) => e,
        Err(err) => {
            on_event(Event::Error(format!("emulator init failed: {err:#}")));
            return;
        }
    };

    let mut next_frame_at = Instant::now();
    let mut pending_frame = false;
    let mut shutdown = false;

    loop {
        // Block until input, PTY data, or the coalesced frame deadline.
        let timeout = if pending_frame {
            let wait = next_frame_at.saturating_duration_since(Instant::now());
            PollTimeout::try_from(wait).unwrap_or(PollTimeout::ZERO)
        } else {
            PollTimeout::NONE
        };
        let pty_fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(pty.as_raw_fd()) };
        let wake_fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(wake_read.as_raw_fd()) };
        let mut fds = [
            PollFd::new(pty_fd, PollFlags::POLLIN | PollFlags::POLLHUP),
            PollFd::new(wake_fd, PollFlags::POLLIN),
        ];
        if let Err(err) = poll(&mut fds, timeout) {
            if err == nix::errno::Errno::EINTR {
                continue;
            }
            on_event(Event::Error(format!("poll failed: {err}")));
            break;
        }
        let pty_flags = fds[0].revents().unwrap_or_else(PollFlags::empty);
        let wake_flags = fds[1].revents().unwrap_or_else(PollFlags::empty);

        // Inputs first: a keystroke should not wait behind a screenful.
        if wake_flags.contains(PollFlags::POLLIN) {
            let mut drain = [0u8; 64];
            while unistd::read(&wake_read, &mut drain).is_ok_and(|n| n > 0) {}
            while let Ok(input) = input_rx.try_recv() {
                if matches!(input, Input::Shutdown) {
                    shutdown = true;
                    continue;
                }
                // The PTY resize goes with the emulator one so the child sees
                // SIGWINCH with matching dimensions.
                if let Input::Resize {
                    cols,
                    rows,
                    cell_w_px,
                    cell_h_px,
                } = input
                {
                    pty.resize(cols, rows, cell_w_px, cell_h_px);
                }
                if let Err(err) = emulator.handle(input) {
                    on_event(Event::Error(format!("input failed: {err:#}")));
                }
            }
        }

        // PTY output; a HUP still carries the last bytes, so read first.
        let mut eof = pty_flags.intersects(PollFlags::POLLHUP | PollFlags::POLLERR);
        if pty_flags.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR) {
            // Cap the bytes consumed per wakeup so a flooding child cannot
            // starve input handling; POLLIN stays raised for the rest.
            const MAX_PER_WAKEUP: usize = 256 * 1024;
            let mut buf = [0u8; 8192];
            let mut consumed = 0usize;
            loop {
                match pty.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        emulator.feed(&buf[..len]);
                        consumed += len;
                        if consumed >= MAX_PER_WAKEUP {
                            break;
                        }
                    }
                    Err(PtyError::EndOfStream) => {
                        eof = true;
                        break;
                    }
                    Err(PtyError::Other(err)) => {
                        on_event(Event::Error(format!("pty read failed: {err}")));
                        eof = true;
                        break;
                    }
                }
            }
        }

        // Terminal answers (DA, kitty queries, OSC) and encoded input.
        let out = emulator.take_output();
        if !out.is_empty() {
            pty.write_all(&out);
        }
        for event in emulator.take_events() {
            on_event(event);
        }

        if shutdown || eof {
            // Deliver the final screen before reporting the exit.
            if emulator.is_dirty() {
                on_event(Event::Frame(Arc::new(emulator.snapshot())));
            }
            let status = if shutdown {
                child.kill();
                None
            } else {
                child.wait_status()
            };
            on_event(Event::Exited { status });
            return;
        }

        // Frame coalescing: a dirty terminal inside the min interval waits for
        // the deadline rather than being dropped. Once deferred, the frame is
        // emitted unconditionally — `is_dirty` is not re-consulted because it
        // consumes the terminal's dirty state.
        let now = Instant::now();
        if pending_frame && now >= next_frame_at {
            on_event(Event::Frame(Arc::new(emulator.snapshot())));
            next_frame_at = now + min_interval;
            pending_frame = false;
        } else if emulator.is_dirty() {
            if now >= next_frame_at {
                on_event(Event::Frame(Arc::new(emulator.snapshot())));
                next_frame_at = now + min_interval;
                pending_frame = false;
            } else {
                pending_frame = true;
            }
        }
    }
}

/// `xterm-ghostty` if terminfo knows it, else the universal fallback.
/// Probed once; the answer cannot change while we run.
fn term_name() -> String {
    static HAS_GHOSTTY: OnceLock<bool> = OnceLock::new();
    let has = *HAS_GHOSTTY.get_or_init(|| {
        std::process::Command::new("infocmp")
            .arg("xterm-ghostty")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    if has {
        "xterm-ghostty"
    } else {
        "xterm-256color"
    }
    .to_string()
}
