//! End-to-end pane test: real PTY, real subprocess, real frames.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use option_term_core::config::Config;
use option_term_vt::{
    emulator::EmulatorOptions,
    graphics::STORAGE_LIMIT,
    input::Event,
    palette::Palette,
    pane::{PaneOptions, spawn},
};

/// `/bin/sh -c 'printf hi; exit 3'` must produce at least one frame showing
/// "hi" and then `Exited { status: Some(3) }`, all within the test timeout.
#[test]
fn pane_runs_a_real_child() {
    let (tx, rx) = mpsc::channel();
    let handle = spawn(
        PaneOptions {
            emulator: EmulatorOptions {
                cols: 80,
                rows: 24,
                cell_w_px: 8,
                cell_h_px: 16,
                scrollback: 1_000,
                palette: Palette::from(&Config::default()),
                cursor_shape: Default::default(),
                cursor_blink: false,
                kitty_storage_limit: STORAGE_LIMIT,
                xtversion: "option-term-vt test".into(),
            },
            cwd: None,
            argv: Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf hi; exit 3".into(),
            ]),
            env: Vec::new(),
            min_frame_interval: Duration::from_millis(8),
        },
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    )
    .expect("spawn pane");
    assert!(handle.child_pid().is_some());

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_hi = false;
    let mut status = None;
    while Instant::now() < deadline {
        let Ok(event) = rx.recv_timeout(Duration::from_millis(500)) else {
            continue;
        };
        match event {
            Event::Frame(frame) => {
                let text: String = frame
                    .lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                saw_hi |= text.contains("hi");
            }
            Event::Exited { status: s } => {
                status = Some(s);
                break;
            }
            Event::Error(e) => panic!("pane error: {e}"),
            _ => {}
        }
    }

    assert!(saw_hi, "no frame ever contained the child's output");
    assert_eq!(status, Some(Some(3)));
}

/// With `min_frame_interval = 200ms` and writes landing ~50ms apart, the
/// coalesced frame must still be emitted at the deadline — never dropped,
/// never spun. Proves the deferred-frame path does not depend on re-reading
/// a consumed dirty flag.
#[test]
fn coalesced_frames_are_not_dropped() {
    fn opts() -> EmulatorOptions {
        EmulatorOptions {
            cols: 80,
            rows: 24,
            cell_w_px: 8,
            cell_h_px: 16,
            scrollback: 1_000,
            palette: Palette::from(&Config::default()),
            cursor_shape: Default::default(),
            cursor_blink: false,
            kitty_storage_limit: STORAGE_LIMIT,
            xtversion: "option-term-vt test".into(),
        }
    }

    let (tx, rx) = mpsc::channel();
    spawn(
        PaneOptions {
            emulator: opts(),
            cwd: None,
            argv: Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf a; sleep 0.05; printf b; sleep 0.05; printf c; sleep 0.3; exit 0".into(),
            ]),
            env: Vec::new(),
            min_frame_interval: Duration::from_millis(200),
        },
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    )
    .expect("spawn pane");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frames: Vec<(Instant, String)> = Vec::new();
    let mut exited = false;
    while Instant::now() < deadline && !exited {
        let Ok(event) = rx.recv_timeout(Duration::from_millis(500)) else {
            continue;
        };
        match event {
            Event::Frame(frame) => {
                let text: String = frame
                    .lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                frames.push((Instant::now(), text));
            }
            Event::Exited { .. } => exited = true,
            Event::Error(e) => panic!("pane error: {e}"),
            _ => {}
        }
    }
    assert!(exited, "pane never exited");

    let last = frames.last().expect("no frames at all").1.clone();
    assert!(last.contains("abc"), "last frame before exit: {last:?}");

    assert!(
        frames
            .iter()
            .any(|(_, t)| t.contains('a') && !t.contains('c')),
        "expected a partial frame before 'c' landed: {frames:?}"
    );

    // No spin: a deferred frame is emitted once at the deadline, not in a
    // zero-timeout poll loop.
    for pair in frames.windows(2) {
        let gap = pair[1].0.duration_since(pair[0].0);
        assert!(
            gap >= Duration::from_millis(150),
            "frames {gap:?} apart — under the 200ms interval: {frames:?}"
        );
    }
}
