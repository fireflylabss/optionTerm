//! Crash handler: persist a readable log when optionTerm panics.
//!
//! A GUI app that dies gives the user no way to report what happened. This
//! installs a panic hook that appends a timestamped entry (version, panic
//! message, backtrace) to `~/.option/terminal/crash.log`, once per process, so
//! the next launch can be inspected. It is best-effort: if the disk or the log
//! path is unusable we fall back to stderr rather than panic again.

use std::{
    fs::OpenOptions,
    io::Write,
    panic::{self, PanicHookInfo},
    path::PathBuf,
    sync::Once,
    time::{SystemTime, UNIX_EPOCH},
};

/// How many frames of the backtrace to keep. A full Rust backtrace of a GTK
/// call chain is huge; the head is the part that names optionTerm symbols.
const MAX_FRAMES: usize = 120;

/// The panic hook is installed once. `option_sdk::App::TERMINAL` gives the
/// `~/.option/terminal` directory shared by the rest of the app.
pub fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = write_crash_log(info);
            // Keep the default hook so the message also lands on stderr and
            // the process still aborts with the usual nonzero status.
            default(info);
        }));
    });
}

/// Append one crash entry. Returns the path written, or `Err` with the reason.
fn write_crash_log(info: &PanicHookInfo) -> std::io::Result<PathBuf> {
    write_crash_log_to(info, crash_log_path())
}

/// Append one crash entry to an explicit path (used by the hook and tests).
fn write_crash_log_to(info: &PanicHookInfo, path: PathBuf) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

    let timestamp = fmt_timestamp(SystemTime::now());
    let version = env!("CARGO_PKG_VERSION");
    let payload = panic_payload(info);
    // Backtrace is captured now, inside the hook, so it reflects this panic
    // (RUST_BACKTRACE is not required; we always ask for one).
    let backtrace = std::backtrace::Backtrace::force_capture();
    let frames = backtrace_to_text(&backtrace);

    writeln!(
        file,
        "===== optionTerm crash log =====\n\
         time:    {timestamp}\n\
         version: {version}\n\
         panic:   {payload}\n\
         --- backtrace ---\n\
         {frames}\n\
         =================================\n"
    )?;
    Ok(path)
}

/// `~/.option/terminal/crash.log`, per optionSDK.
fn crash_log_path() -> PathBuf {
    option_sdk::App::TERMINAL.dir().join("crash.log")
}

/// The panic location (file:line) plus the message, or the raw payload.
fn panic_payload(info: &PanicHookInfo) -> String {
    let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    };
    match info.location() {
        Some(loc) => format!("{loc} — {msg}"),
        None => msg,
    }
}

/// Render a `Backtrace` as a `Debug` string, capped to the first `MAX_FRAMES`
/// lines so a runaway stack does not fill the log.
fn backtrace_to_text(bt: &std::backtrace::Backtrace) -> String {
    let mut out = format!("{bt:?}");
    if let Some(idx) = out.find("\nAt ") {
        // Trim the trailing symbol-resolution note that rust adds after the
        // frames; it repeats the same addresses and bloats the log.
        out.truncate(idx);
    }
    // Keep only the head of the stack.
    let lines = out.lines().take(MAX_FRAMES).collect::<Vec<_>>();
    lines.join("\n")
}

/// Compact `YYYY-MM-DD HH:MM:SS` from a `SystemTime`.
fn fmt_timestamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    // days + seconds since epoch → civil date (no external crate).
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

/// Convert days-since-epoch to a civil (year, month, day). Howard Hinnant's
/// `civil_from_days` algorithm, originally for `date`/`std::chrono`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_known_dates() {
        // 1970-01-01, 2026-08-24, 2000-02-29 (leap day).
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_689), (2026, 8, 24));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn timestamp_format() {
        let s = fmt_timestamp(SystemTime::UNIX_EPOCH);
        assert_eq!(s, "1970-01-01 00:00:00");
    }

    /// The writer records a panic message into the path it is given; point it at
    /// a temp dir so the test never touches the user's real crash log.
    #[test]
    fn crash_log_writes_message() {
        let Some(path) = std::env::var_os("OPTIONTERM_CRASH_TEST_PATH") else {
            let dir = crate::test_support::TestDir::new("crash-log");
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "crash::tests::crash_log_writes_message"])
                .env("OPTIONTERM_CRASH_TEST_PATH", dir.path().join("crash.log"))
                .status()
                .unwrap();
            assert!(status.success());
            return;
        };
        let path = PathBuf::from(path);

        // Build a PanicHookInfo by catching a real panic with our writer as the
        // hook; the message must land in `path`.
        let hook = panic::take_hook();
        let saved_path = path.clone();
        panic::set_hook(Box::new(move |info| {
            let _ = write_crash_log_to(info, saved_path.clone());
        }));
        let res = panic::catch_unwind(|| {
            panic!("deliberate crash-log test");
        });
        panic::set_hook(hook);
        assert!(res.is_err());

        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            text.contains("deliberate crash-log test"),
            "crash log should contain the panic message; got: {text:?}"
        );
    }
}
