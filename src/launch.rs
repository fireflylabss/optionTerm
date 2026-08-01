//! CLI launch options (`--working-directory`, `-e` / `--`).

use std::path::PathBuf;

/// How a new tab / first window should spawn its PTY.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    pub cwd: Option<PathBuf>,
    /// When set, spawn this argv instead of the login shell.
    pub command: Option<Vec<String>>,
}

impl LaunchRequest {
    pub fn is_default(&self) -> bool {
        self.cwd.is_none() && self.command.is_none()
    }
}

/// Parse `optionterm` argv (including argv[0]).
///
/// Supported:
/// - `--working-directory DIR` / `-d DIR` / `--working-directory=DIR`
/// - `-e CMD [ARGS…]` — everything after `-e` is the command
/// - `-- CMD [ARGS…]` — same, GNU end-of-options form
/// - a single positional path that is an existing directory becomes cwd
pub fn parse_args<S: AsRef<str>>(args: &[S]) -> LaunchRequest {
    let mut cwd = None;
    let mut command = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1; // skip argv0
    let args: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();

    while i < args.len() {
        let arg = args[i];
        if arg == "--" {
            let rest: Vec<String> = args[i + 1..].iter().map(|s| (*s).to_string()).collect();
            if !rest.is_empty() {
                command = Some(rest);
            }
            break;
        }
        if arg == "-e" || arg == "--command" || arg == "-x" {
            let rest: Vec<String> = args[i + 1..].iter().map(|s| (*s).to_string()).collect();
            if !rest.is_empty() {
                command = Some(rest);
            }
            break;
        }
        if let Some(dir) = arg.strip_prefix("--working-directory=") {
            cwd = Some(PathBuf::from(dir));
            i += 1;
            continue;
        }
        if arg == "--working-directory" || arg == "-d" || arg == "--workdir" {
            if let Some(next) = args.get(i + 1) {
                cwd = Some(PathBuf::from(next));
                i += 2;
                continue;
            }
        }
        if arg == "-h" || arg == "--help" {
            // Handled by the caller printing help; ignore here.
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            // Unknown flag — skip so GApplication / future flags don't break.
            i += 1;
            continue;
        }
        positional.push(arg.to_string());
        i += 1;
    }

    if cwd.is_none() {
        for p in &positional {
            let path = PathBuf::from(p);
            if path.is_dir() {
                cwd = Some(path);
                break;
            }
        }
    }

    LaunchRequest { cwd, command }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_working_directory_and_command() {
        let req = parse_args(&[
            "optionterm",
            "--working-directory",
            "/tmp",
            "-e",
            "htop",
            "-t",
        ]);
        assert_eq!(req.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(req.command, Some(vec!["htop".into(), "-t".into()]));
    }

    #[test]
    fn parses_double_dash_command() {
        let req = parse_args(&["optionterm", "--", "vim", "file.rs"]);
        assert_eq!(req.command, Some(vec!["vim".into(), "file.rs".into()]));
    }

    #[test]
    fn equals_form_for_workdir() {
        let req = parse_args(&["optionterm", "--working-directory=/home/u"]);
        assert_eq!(req.cwd, Some(PathBuf::from("/home/u")));
        assert!(req.command.is_none());
    }
}
