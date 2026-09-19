//! URI detection for click-to-open: OSC 8 aside, bare URLs and paths in
//! terminal output.
//!
//! Ported from optionTerm 0.1.x, minus the `gio::File` dependency:
//! `pwd_to_path` decodes `file://` URIs itself.

/// Decode the shell's OSC 7 report into a plain filesystem path.
///
/// Shells emit `file://<host>/<percent-encoded path>`; the host part must be
/// dropped and the path unescaped, otherwise a `cd` would land in something
/// like `file://myhost/home/me`.
pub fn pwd_to_path(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    // Skip the host: the path is the first `/` onward (empty = "/").
    let path = match rest.find('/') {
        Some(slash) => &rest[slash..],
        None => "/",
    };
    percent_decode(path)
}

fn percent_decode(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Turn a word grabbed from the screen into an openable URI.
///
/// Recognises explicit schemes, bare `www.` hosts, and filesystem paths that
/// actually exist (relative ones resolved against the shell's reported pwd).
/// Returns `None` for ordinary words so a modified click stays a no-op on prose.
pub fn detect_link(word: &str, pwd: Option<&str>) -> Option<String> {
    // Terminal output is full of trailing punctuation: `see https://x.dev.`
    let word = word.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\''
            )
    });
    let word = word.trim_end_matches(['.', ':']);
    if word.is_empty() {
        return None;
    }

    const SCHEMES: &[&str] = &[
        "http://", "https://", "ftp://", "file://", "mailto:", "ssh://", "git://",
    ];
    if SCHEMES.iter().any(|s| word.starts_with(s)) {
        return Some(word.to_string());
    }
    if let Some(rest) = word.strip_prefix("www.")
        && rest.contains('.')
    {
        return Some(format!("https://{word}"));
    }
    // A bare `user@host` reads as an email address.
    if !word.contains('/') && word.matches('@').count() == 1 {
        let (user, host) = word.split_once('@')?;
        if !user.is_empty() && host.contains('.') && !host.starts_with('.') {
            return Some(format!("mailto:{word}"));
        }
    }

    // Paths: only offer them when they resolve to something real, otherwise
    // every dotted word would look like a file.
    let expanded = match word.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None => std::path::PathBuf::from(word),
    };
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        std::path::PathBuf::from(pwd.filter(|p| !p.is_empty())?).join(expanded)
    };
    candidate
        .exists()
        .then(|| format!("file://{}", candidate.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_uri() {
        assert_eq!(pwd_to_path("file://host/tmp/x"), "/tmp/x");
        assert_eq!(pwd_to_path("file:///tmp/x"), "/tmp/x");
        assert_eq!(pwd_to_path("file://h/a%20b"), "/a b");
        assert_eq!(pwd_to_path("file://host"), "/");
        assert_eq!(pwd_to_path("/plain/path"), "/plain/path");
    }

    #[test]
    fn detects_schemes_and_www() {
        assert_eq!(
            detect_link("https://example.com.", None).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            detect_link("www.example.com/x", None).as_deref(),
            Some("https://www.example.com/x")
        );
        assert_eq!(
            detect_link("me@example.com", None).as_deref(),
            Some("mailto:me@example.com")
        );
        assert_eq!(detect_link("justaword", None), None);
    }
}
