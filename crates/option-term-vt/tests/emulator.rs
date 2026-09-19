//! Headless emulator tests — no PTY, no UI toolkit.

use option_term_core::config::Config;
use option_term_vt::{
    emulator::{Emulator, EmulatorOptions},
    frame::Style,
    graphics::STORAGE_LIMIT,
    input::{
        Event, Input, Key, KeyAction, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind, NamedKey,
        SelectKind,
    },
    palette::Palette,
};

fn emulator(cols: u16, rows: u16) -> Emulator {
    Emulator::new(EmulatorOptions {
        cols,
        rows,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback: 10_000,
        palette: Palette::from(&Config::default()),
        cursor_shape: Default::default(),
        cursor_blink: true,
        kitty_storage_limit: STORAGE_LIMIT,
        xtversion: "option-term-vt test".into(),
    })
    .expect("emulator")
}

fn key(action: KeyAction, k: Key, mods: Mods, text: Option<&str>) -> Input {
    Input::Key(KeyEvent {
        action,
        key: k,
        mods,
        text: text.map(String::from),
        unshifted: None,
    })
}

/// Minimal base64 encoder for Kitty payloads.
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[test]
fn sgr_runs_carry_colors_and_flags() {
    let mut em = emulator(80, 24);
    em.feed(b"Hello, \x1b[1;32mworld\x1b[0m!");
    let frame = em.snapshot();

    assert_eq!(frame.cols, 80);
    assert_eq!(frame.rows, 24);
    assert_eq!(frame.lines.len(), 24);

    let runs = &frame.lines[0].runs;
    // The blank cell between "Hello," and "world" carries no content: the
    // run collector drops it at the pen boundary and "world" starts at col 7.
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].text, "Hello,");
    assert_eq!(runs[0].col, 0);
    assert_eq!(runs[2].text, "!");

    let world = runs
        .iter()
        .find(|r| r.text.contains("world"))
        .expect("world run");
    assert!(world.style.contains(Style::BOLD), "world must be bold");
    // libghostty resolves `1;32` to bright green (bold-is-bright), i.e.
    // palette index 10 rather than plain green (2).
    let palette = Palette::from(&Config::default());
    assert_eq!(world.fg, palette.colors[10]);

    let plain = runs.iter().find(|r| r.text.contains("Hello")).unwrap();
    assert!(!plain.style.contains(Style::BOLD));
    assert_eq!(plain.fg, frame.default_fg);
}

#[test]
fn truecolor_and_wide_chars() {
    let mut em = emulator(80, 24);
    em.feed(b"\x1b[38;2;255;128;0m\x1b[48;2;1;2;3mX\xe6\xbc\xa2Y");
    let frame = em.snapshot();
    let runs = &frame.lines[0].runs;

    let x = runs.iter().find(|r| r.text.contains('X')).expect("X run");
    let rgb = |r, g, b| option_term_vt::frame::Rgb { r, g, b };
    assert_eq!(x.fg, rgb(255, 128, 0));
    assert_eq!(x.bg, Some(rgb(1, 2, 3)));

    let han = runs.iter().find(|r| r.text.contains('漢')).expect("漢 run");
    assert_eq!(han.cells, 2, "wide char covers two cells");
    assert_eq!(han.col, 1);
    // The spacer tail at col 2 produced no run of its own.
    assert!(runs.iter().all(|r| r.col != 2 || r.text.contains('漢')));
}

#[test]
fn is_dirty_is_sticky_until_snapshot() {
    // RenderState::update consumes the terminal's dirty flag; `is_dirty`
    // latches a positive answer so coalescing callers can poll repeatedly.
    let mut em = emulator(80, 24);
    em.feed(b"x");
    assert!(em.is_dirty());
    assert!(em.is_dirty());
    let _ = em.snapshot();
    assert!(!em.is_dirty());
}

#[test]
fn kitty_query_and_da_answer_on_the_output() {
    let mut em = emulator(80, 24);
    em.feed(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    let out = em.take_output();
    assert!(
        out.windows(12).any(|w| w == b"\x1b_Gi=31;OK\x1b\\"),
        "kitty query response missing: {out:?}"
    );

    em.feed(b"\x1b[c");
    let out = em.take_output();
    assert!(
        out.windows(3).any(|w| w == b"\x1b[?"),
        "DA1 response missing: {out:?}"
    );
}

#[test]
fn kitty_rgba_transmit_and_place() {
    let mut em = emulator(80, 24);
    let px: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let cmd = format!("\x1b_Ga=T,f=32,t=d,i=1,s=2,v=2,q=2;{}\x1b\\", b64(&px));
    em.feed(cmd.as_bytes());
    let frame = em.snapshot();

    assert_eq!(frame.kitty.placements.len(), 1);
    let p = &frame.kitty.placements[0];
    assert_eq!(p.image_id, 1);
    assert_eq!(p.col, 0);
    assert_eq!(p.row, 0);
    // No c=/r= given: the placement renders at native pixel size.
    assert_eq!(p.dest_px, (2, 2));

    assert_eq!(frame.kitty.images.len(), 1);
    let img = &frame.kitty.images[0];
    assert_eq!(img.id, 1);
    assert_eq!((img.width, img.height), (2, 2));
    assert_eq!(&*img.rgba, &px);

    // Already sent this generation: the second snapshot ships no pixels.
    let frame2 = em.snapshot();
    assert!(frame2.kitty.images.is_empty());
    assert_eq!(frame2.kitty.placements.len(), 1);
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }
    out
}

#[test]
fn kitty_png_is_decoded() {
    let mut em = emulator(80, 24);
    // 4x3: top row red, middle green, bottom blue.
    let mut rgba = Vec::new();
    for row in 0..3 {
        for _ in 0..4 {
            rgba.extend_from_slice(match row {
                0 => &[255, 0, 0, 255],
                1 => &[0, 255, 0, 255],
                _ => &[0, 0, 255, 255],
            });
        }
    }
    let png = encode_png(4, 3, &rgba);
    let cmd = format!("\x1b_Ga=T,f=100,t=d,i=7,q=2;{}\x1b\\", b64(&png));
    em.feed(cmd.as_bytes());
    let frame = em.snapshot();

    let img = frame
        .kitty
        .images
        .iter()
        .find(|i| i.id == 7)
        .expect("decoded PNG in the frame");
    assert_eq!((img.width, img.height), (4, 3));
    assert_eq!(img.rgba.len(), 4 * 3 * 4);
    assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&img.rgba[16..20], &[0, 255, 0, 255]);
}

#[test]
fn kitty_chunked_zlib() {
    let mut em = emulator(80, 24);
    // 8x8 RGB, one gradient.
    let mut rgb = Vec::with_capacity(8 * 8 * 3);
    for y in 0..8u8 {
        for x in 0..8u8 {
            rgb.extend_from_slice(&[x * 32, y * 32, 128]);
        }
    }
    let compressed = {
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&rgb).unwrap();
        enc.finish().unwrap()
    };
    let encoded = b64(&compressed);
    for (i, chunk) in encoded.as_bytes().chunks(64).enumerate() {
        let last = (i + 1) * 64 >= encoded.len();
        let chunk = std::str::from_utf8(chunk).unwrap();
        let cmd = format!(
            "\x1b_Ga=T,f=24,o=z,t=d,i=2,s=8,v=8,q=2,m={};{}\x1b\\",
            u8::from(!last),
            chunk
        );
        em.feed(cmd.as_bytes());
    }
    let frame = em.snapshot();
    let img = frame
        .kitty
        .images
        .iter()
        .find(|i| i.id == 2)
        .expect("chunked zlib image");
    assert_eq!((img.width, img.height), (8, 8));
    // RGB was expanded to RGBA.
    assert_eq!(img.rgba.len(), 8 * 8 * 4);
    assert_eq!(&img.rgba[..4], &[0, 0, 128, 255]);
    // Pixel (7,0) → offset 7*4.
    assert_eq!(&img.rgba[28..32], &[7 * 32, 0, 128, 255]);
}

#[test]
fn kitty_file_mediums() {
    let mut em = emulator(80, 24);
    let dir = tempfile::tempdir().unwrap();

    // t=f: the terminal reads the file; it stays on disk.
    let persistent = dir.path().join("img.rgba");
    std::fs::write(
        &persistent,
        [9u8, 8, 7, 6, 5, 4, 3, 2, 1, 0, 255, 128, 64, 32, 16, 8],
    )
    .unwrap();
    let cmd = format!(
        "\x1b_Ga=T,f=32,t=f,i=3,s=2,v=2,q=2;{}\x1b\\",
        b64(persistent.to_str().unwrap().as_bytes())
    );
    em.feed(cmd.as_bytes());
    let frame = em.snapshot();
    assert!(
        frame.kitty.images.iter().any(|i| i.id == 3 && i.width == 2),
        "t=f image missing"
    );
    assert!(persistent.exists(), "t=f must not delete the source");

    // t=t (temporary file, deleted after read) is rejected by this binding
    // revision: libghostty-vt answers `EINVAL: unsupported medium` and leaves
    // the file alone. Verified empirically; keep the check so a future
    // binding update that adds the medium is loud here.
    let tmp = dir.path().join("tmp.rgba");
    std::fs::write(&tmp, [1u8; 16]).unwrap();
    let cmd = format!(
        "\x1b_Ga=T,f=32,t=t,i=4,s=2,v=2;{}\x1b\\",
        b64(tmp.to_str().unwrap().as_bytes())
    );
    em.feed(cmd.as_bytes());
    let out = String::from_utf8_lossy(&em.take_output()).into_owned();
    assert!(
        out.contains("EINVAL"),
        "expected unsupported-medium response, got {out:?}"
    );
    assert!(tmp.exists());
}

#[test]
fn kitty_shared_mem() {
    let mut em = emulator(80, 24);
    let path = format!("/dev/shm/otvt-{}", std::process::id());
    if std::fs::write(&path, [42u8; 16]).is_err() {
        eprintln!("skipping: /dev/shm not writable");
        return;
    }
    let cmd = format!(
        "\x1b_Ga=T,f=32,t=s,i=5,s=2,v=2,q=2;{}\x1b\\",
        b64(path.trim_start_matches("/dev/shm").as_bytes())
    );
    em.feed(cmd.as_bytes());
    let frame = em.snapshot();
    let _ = std::fs::remove_file(&path);
    assert!(frame.kitty.images.iter().any(|i| i.id == 5));
}

#[test]
fn kitty_delete_reports_dropped() {
    let mut em = emulator(80, 24);
    let px = [255u8; 16];
    em.feed(format!("\x1b_Ga=T,f=32,t=d,i=1,s=2,v=2,q=2;{}\x1b\\", b64(&px)).as_bytes());
    assert_eq!(em.snapshot().kitty.placements.len(), 1);

    // Verified on libghostty-vt rev 5988a0b: `d=i` removes only the
    // placement and keeps the image data — a bare `a=p,i=1` re-places it.
    // `d=I` frees the image itself, which is what surfaces in `dropped`.
    // (This is the inverse of the letter-casing semantics in the kitty spec.)
    em.feed(b"\x1b_Ga=d,d=i,i=1,q=2\x1b\\");
    let frame = em.snapshot();
    assert!(frame.kitty.placements.is_empty());
    assert!(frame.kitty.dropped.is_empty());

    em.feed(b"\x1b_Ga=p,i=1,q=2\x1b\\");
    assert_eq!(
        em.snapshot().kitty.placements.len(),
        1,
        "image data survived d=i"
    );

    em.feed(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\");
    let frame = em.snapshot();
    assert!(frame.kitty.placements.is_empty());
    assert!(
        frame.kitty.dropped.contains(&1),
        "dropped: {:?}",
        frame.kitty.dropped
    );
}

#[test]
fn resize_keeps_content() {
    let mut em = emulator(80, 24);
    for i in 0..30 {
        em.feed(format!("line {i}\r\n").as_bytes());
    }
    em.handle(Input::Resize {
        cols: 40,
        rows: 10,
        cell_w_px: 8,
        cell_h_px: 16,
    })
    .unwrap();
    let frame = em.snapshot();
    assert_eq!(frame.cols, 40);
    assert_eq!(frame.rows, 10);
    assert_eq!(frame.lines.len(), 10);
    assert!(frame.scroll.total > 0, "scrollback must exist after shrink");
}

#[test]
fn search_finds_wraps_and_clears() {
    let mut em = emulator(80, 24);
    for i in 0..50 {
        em.feed(format!("line {i}\r\n").as_bytes());
    }
    // "line 3" hits "line 3" and "line 30".."line 39": 11 matches.
    em.handle(Input::Search(Some("line 3".into()))).unwrap();
    let frame = em.snapshot();
    assert_eq!(frame.matches.len(), 11);
    assert_eq!(frame.active_match, Some(0));

    em.handle(Input::SearchNext).unwrap();
    assert_eq!(em.snapshot().active_match, Some(1));
    // SearchPrev wraps backwards past the first match.
    em.handle(Input::SearchPrev).unwrap();
    em.handle(Input::SearchPrev).unwrap();
    assert_eq!(em.snapshot().active_match, Some(10));
    // SearchNext wraps past the last match.
    em.handle(Input::SearchNext).unwrap();
    assert_eq!(em.snapshot().active_match, Some(0));

    em.handle(Input::Search(None)).unwrap();
    let frame = em.snapshot();
    assert!(frame.matches.is_empty());
    assert_eq!(frame.active_match, None);
}

#[test]
fn osc7_pwd_and_osc0_title() {
    let mut em = emulator(80, 24);
    em.feed(b"\x1b]7;file://host/tmp/x\x1b\\");
    em.feed(b"\x1b]0;my title\x07");
    let frame = em.snapshot();
    assert_eq!(frame.pwd.as_deref(), Some("/tmp/x"));
    assert_eq!(frame.title, "my title");
    let events = em.take_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Title(t) if t == "my title"))
    );
}

#[test]
fn key_encoding() {
    let mut em = emulator(80, 24);

    em.handle(key(
        KeyAction::Press,
        Key::Char('a'),
        Mods {
            ctrl: true,
            ..Default::default()
        },
        Some("a"),
    ))
    .unwrap();
    assert_eq!(em.take_output(), vec![0x01]);

    em.handle(key(
        KeyAction::Press,
        Key::Named(NamedKey::Enter),
        Mods::default(),
        Some("\r"),
    ))
    .unwrap();
    assert_eq!(em.take_output(), b"\r");

    // Cursor keys: CSI normally, SS3 in application mode (DECCKM).
    em.handle(key(
        KeyAction::Press,
        Key::Named(NamedKey::Up),
        Mods::default(),
        None,
    ))
    .unwrap();
    assert_eq!(em.take_output(), b"\x1b[A");

    em.feed(b"\x1b[?1h");
    em.handle(key(
        KeyAction::Press,
        Key::Named(NamedKey::Up),
        Mods::default(),
        None,
    ))
    .unwrap();
    assert_eq!(em.take_output(), b"\x1bOA");

    // Bracketed paste wraps the payload.
    em.feed(b"\x1b[?2004h");
    em.handle(Input::Paste("ab".into())).unwrap();
    assert_eq!(em.take_output(), b"\x1b[200~ab\x1b[201~");
}

#[test]
fn word_selection_and_copy() {
    let mut em = emulator(80, 24);
    em.feed(b"hello world");

    em.handle(Input::SelectionBegin {
        col: 0,
        row: 0,
        kind: SelectKind::Word,
        rectangle: false,
    })
    .unwrap();
    em.handle(Input::CopySelection).unwrap();
    let events = em.take_events();
    let text = events.iter().find_map(|e| match e {
        Event::SelectionText(t) => t.clone(),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("hello"));

    em.handle(Input::SelectAll).unwrap();
    em.handle(Input::CopySelection).unwrap();
    let events = em.take_events();
    let text = events.iter().find_map(|e| match e {
        Event::SelectionText(t) => t.clone(),
        _ => None,
    });
    assert!(text.as_deref().unwrap_or_default().contains("hello world"));
}

#[test]
fn mouse_reporting_sgr() {
    let mut em = emulator(80, 24);
    em.feed(b"\x1b[?1000h\x1b[?1006h");
    let frame = em.snapshot();
    assert!(frame.modes.mouse_reporting);

    em.handle(Input::Mouse(MouseEvent {
        kind: MouseKind::Press(MouseButton::Left),
        col: 4,
        row: 2,
        px_in_cell: (0, 0),
        mods: Mods::default(),
    }))
    .unwrap();
    // SGR: button 0, col+1=5, row+1=3.
    assert_eq!(em.take_output(), b"\x1b[<0;5;3M");
}
