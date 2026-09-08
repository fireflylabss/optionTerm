use gtk4::{gio, glib, prelude::*};
use libadwaita as adw;
use std::{
    cell::Cell,
    io::Read,
    os::fd::IntoRawFd,
    rc::Rc,
    time::{Duration, Instant},
};
use vte4::prelude::*;
use webkit6::prelude::*;

pub fn run() -> anyhow::Result<()> {
    let app = adw::Application::builder()
        .application_id("io.option.terminal.selftest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let success = Rc::new(Cell::new(false));
    let finished = success.clone();
    app.connect_activate(move |app| {
        if let Err(error) = open_probe(app, finished.clone()) {
            eprintln!("self-test initialization failed: {error}");
            app.quit();
        }
    });
    app.run_with_args(&["optionterm-self-test"]);
    anyhow::ensure!(
        success.get(),
        "self-test failed: GTK, Kitty protocol or WebKit did not complete"
    );
    println!("optionterm self-test: GTK=OK Kitty=OK WebKit=OK");
    Ok(())
}

fn open_probe(app: &adw::Application, success: Rc<Cell<bool>>) -> anyhow::Result<()> {
    let (terminal, mut slave) = kitty_probe()?;
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);
    let web = webkit6::WebView::builder()
        .network_session(&webkit6::NetworkSession::new_ephemeral())
        .build();
    web.set_vexpand(true);
    web.set_size_request(320, 160);
    web.connect_load_failed(|_, event, _, error| {
        eprintln!("WebKit self-test load failed at {event:?}: {error}");
        false
    });
    web.connect_web_process_terminated(|_, reason| {
        eprintln!("WebKit self-test process terminated: {reason:?}")
    });
    let web_ready = Rc::new(Cell::new(false));
    let ready = web_ready.clone();
    web.connect_load_changed(move |web, event| {
        if event == webkit6::LoadEvent::Finished
            && web.title().as_deref() == Some("optionterm-self-test")
        {
            ready.set(true);
        }
    });
    let ready = web_ready.clone();
    web.connect_title_notify(move |web| {
        if web.title().as_deref() == Some("optionterm-self-test") {
            ready.set(true);
        }
    });
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.append(&terminal);
    root.append(&web);
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(640)
        .default_height(480)
        .title("optionTerm verification")
        .content(&root)
        .build();
    web.load_html(
        "<!doctype html><title>optionterm-self-test</title><p>WebKit verification</p>",
        Some("about:blank"),
    );
    window.present();
    let started = Instant::now();
    let app = app.clone();
    let window = window.downgrade();
    let web = web.downgrade();
    let mut reply = Vec::new();
    glib::timeout_add_local(Duration::from_millis(25), move || {
        let mut bytes = [0u8; 2048];
        loop {
            match slave.read(&mut bytes) {
                Ok(0) => break,
                Ok(size) => reply.extend_from_slice(&bytes[..size]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        let reply_text = String::from_utf8_lossy(&reply);
        let kitty_ready = reply_text.contains("Gi=7;OK")
            && reply_text.contains("Gi=8;OK")
            && reply_text.contains("Gi=9;")
            && !reply_text.contains("Gi=9;OK");
        if kitty_ready && web_ready.get() {
            success.set(true);
        }
        if success.get()
            || started.elapsed() >= Duration::from_secs(20)
            || window.upgrade().is_none()
        {
            if !success.get() {
                if let Some(web) = web.upgrade() {
                    eprintln!(
                        "WebKit probe state: title={:?} loading={} mapped={}",
                        web.title(),
                        web.is_loading(),
                        web.is_mapped()
                    );
                }
                eprintln!(
                    "self-test incomplete: Kitty={kitty_ready} WebKit={}",
                    web_ready.get()
                );
            }
            if let Some(window) = window.upgrade() {
                window.close();
            }
            app.quit();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    Ok(())
}

pub(crate) fn kitty_probe() -> anyhow::Result<(vte4::Terminal, std::fs::File)> {
    let winsize = nix::pty::Winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 640,
        ws_ypixel: 384,
    };
    let nix::pty::OpenptyResult { master, slave } = nix::pty::openpty(&winsize, None)?;
    let mut termios = nix::sys::termios::tcgetattr(&slave)?;
    nix::sys::termios::cfmakeraw(&mut termios);
    nix::sys::termios::tcsetattr(&slave, nix::sys::termios::SetArg::TCSANOW, &termios)?;
    let flags =
        nix::fcntl::OFlag::from_bits_retain(nix::fcntl::fcntl(&slave, nix::fcntl::F_GETFL)?)
            | nix::fcntl::OFlag::O_NONBLOCK;
    nix::fcntl::fcntl(&slave, nix::fcntl::F_SETFL(flags))?;
    unsafe extern "C" {
        fn vte_pty_new_foreign_sync(
            fd: i32,
            cancellable: *mut std::ffi::c_void,
            error: *mut *mut glib::ffi::GError,
        ) -> *mut vte4::ffi::VtePty;
    }
    let pty: vte4::Pty = unsafe {
        let mut error = std::ptr::null_mut();
        let pointer =
            vte_pty_new_foreign_sync(master.into_raw_fd(), std::ptr::null_mut(), &mut error);
        if !error.is_null() {
            return Err(glib::translate::from_glib_full::<_, glib::Error>(error).into());
        }
        anyhow::ensure!(!pointer.is_null(), "failed to create verification PTY");
        glib::translate::from_glib_full(pointer)
    };
    let terminal = vte4::Terminal::new();
    terminal.set_pty(Some(&pty));
    terminal.set_enable_sixel(true);
    terminal.feed(b"\x1b_Gi=7,a=q,t=d,f=24,s=1,v=1;AAAA\x1b\\");
    terminal.feed(b"\x1b_Gi=8,a=T,t=d,f=32,s=1,v=1,o=z,m=1;eJxjYGBg\x1b\\\x1b_Gm=0;AAAABAAB\x1b\\");
    terminal.feed(b"\x1b_Gi=9,a=q,t=d,f=99,s=1,v=1;AAAA\x1b\\");
    Ok((terminal, std::fs::File::from(slave)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gtk4::test]
    fn kitty_probe_checks_rgb_compressed_chunks_and_errors() {
        let (_terminal, mut slave) = kitty_probe().unwrap();
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            let mut bytes = [0u8; 2048];
            if let Ok(size) = slave.read(&mut bytes) {
                received.extend_from_slice(&bytes[..size]);
            }
            let reply = String::from_utf8_lossy(&received);
            if reply.contains("Gi=7;OK")
                && reply.contains("Gi=8;OK")
                && reply.contains("Gi=9;")
                && !reply.contains("Gi=9;OK")
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "incomplete Kitty protocol responses: {:?}",
            String::from_utf8_lossy(&received)
        );
    }
}
