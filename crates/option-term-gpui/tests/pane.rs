//! GPUI-level tests for `Pane`, driving it through a channel (no PTY).

use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{AppContext, TestAppContext};
use option_term_core::config::Config;
use option_term_gpui::pane::Pane;
use option_term_vt::frame::{
    Cursor, CursorShape, Frame, Kitty, Line, Match, Modes, Rgb, Run, Scroll, Style,
};
use option_term_vt::input::Event;

fn frame_with_text(text: &str) -> Arc<Frame> {
    Arc::new(Frame {
        cols: 80,
        rows: 24,
        lines: vec![Line {
            runs: vec![Run {
                col: 0,
                cells: text.len() as u16,
                text: text.to_string(),
                fg: Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
                bg: None,
                underline_color: None,
                style: Style::empty(),
                selected: false,
                hyperlink: false,
            }],
        }],
        cursor: Some(Cursor {
            col: 0,
            row: 0,
            shape: CursorShape::Block,
            color: Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            text_color: Rgb { r: 0, g: 0, b: 0 },
            blinking: false,
        }),
        scroll: Scroll {
            offset: 0,
            total: 0,
        },
        modes: Modes {
            mouse_reporting: false,
            bracketed_paste: false,
            alt_screen: false,
            focus_events: false,
            kitty_keyboard: false,
        },
        title: String::new(),
        pwd: None,
        kitty: Kitty {
            generation: 0,
            placements: Vec::new(),
            images: Vec::new(),
            dropped: Vec::new(),
        },
        matches: Vec::<Match>::new(),
        active_match: None,
        default_bg: Rgb { r: 0, g: 0, b: 0 },
        default_fg: Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        seq: 1,
    })
}

#[gpui::test]
async fn pane_applies_frames(cx: &mut TestAppContext) {
    let (tx, rx) = mpsc::unbounded();
    let config = Config::default();
    let pane = cx.new(|cx| Pane::with_channel(&config, None, rx, cx));

    tx.unbounded_send(Event::Frame(frame_with_text("hi")))
        .expect("send frame");
    cx.run_until_parked();

    pane.read_with(cx, |pane, _| {
        assert_eq!(pane.frame().lines[0].runs[0].text, "hi");
    });

    tx.unbounded_send(Event::Title("x".to_string()))
        .expect("send title");
    cx.run_until_parked();

    pane.read_with(cx, |pane, _| {
        assert_eq!(pane.title(), "x");
    });
}
