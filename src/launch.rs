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
        if (arg == "--working-directory" || arg == "-d" || arg == "--workdir")
            && let Some(next) = args.get(i + 1)
        {
            cwd = Some(PathBuf::from(next));
            i += 2;
            continue;
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

pub const HELP: &str = "Usage: optionterm [OPTIONS] [DIRECTORY]\n\nOptions:\n  -d, --working-directory DIR   Start in DIR\n  -e, --command CMD [ARGS…]     Run CMD instead of the shell\n  -- CMD [ARGS…]                Same as -e\n  -h, --help                    Show this help\n  --version                     Show version\n  --self-test                   Verify GTK, Kitty and WebKit (needs a display)\n";

#[derive(Debug, PartialEq, Eq)]
pub enum CommandLine {
    Help,
    Version,
    SelfTest,
    Launch(LaunchRequest),
}

pub fn parse_command_line<S: AsRef<str>>(
    args: &[S],
    caller_cwd: &std::path::Path,
) -> anyhow::Result<CommandLine> {
    let mut normalized: Vec<String> = args.iter().map(|s| s.as_ref().to_string()).collect();
    let mut index = 1;
    let mut directory = false;
    while index < normalized.len() {
        let argument = normalized[index].clone();
        match argument.as_str() {
            "--" | "-e" | "--command" | "-x" => {
                anyhow::ensure!(
                    normalized.get(index + 1).is_some_and(|s| !s.is_empty()),
                    "missing command after {argument}"
                );
                anyhow::ensure!(
                    normalized[index + 1..].iter().all(|s| !s.contains('\0')),
                    "command arguments must not contain NUL"
                );
                break;
            }
            "--help" | "-h" => return Ok(CommandLine::Help),
            "--version" => return Ok(CommandLine::Version),
            "--self-test" => return Ok(CommandLine::SelfTest),
            "-d" | "--working-directory" | "--workdir" => {
                let next = normalized
                    .get_mut(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing directory after {argument}"))?;
                *next = launch_directory(next, caller_cwd)?
                    .to_string_lossy()
                    .into_owned();
                directory = true;
                index += 2;
                continue;
            }
            _ => {}
        }
        if let Some(path) = argument.strip_prefix("--working-directory=") {
            normalized[index] = format!(
                "--working-directory={}",
                launch_directory(path, caller_cwd)?.display()
            );
            directory = true;
        } else {
            anyhow::ensure!(!argument.starts_with('-'), "unknown option: {argument}");
            anyhow::ensure!(!directory, "only one working directory can be specified");
            normalized[index] = launch_directory(&argument, caller_cwd)?
                .to_string_lossy()
                .into_owned();
            directory = true;
        }
        index += 1;
    }
    let mut launch = parse_args(&normalized);
    if launch.command.is_some() && launch.cwd.is_none() {
        launch.cwd = Some(caller_cwd.to_path_buf());
    }
    Ok(CommandLine::Launch(launch))
}

fn launch_directory(path: &str, base: &std::path::Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !path.is_empty() && !path.contains('\0'),
        "invalid working directory"
    );
    let path = base.join(path);
    anyhow::ensure!(
        path.is_dir(),
        "working directory does not exist or is not accessible"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_help_and_empty_arguments_are_forwarded() {
        let CommandLine::Launch(request) = parse_command_line(
            &["optionterm", "-e", "program", "--help", ""],
            std::path::Path::new("/tmp"),
        )
        .unwrap() else {
            panic!("expected launch")
        };
        assert_eq!(request.command.unwrap(), ["program", "--help", ""]);
        assert_eq!(request.cwd.unwrap(), PathBuf::from("/tmp"));
        assert_eq!(
            parse_command_line(&["optionterm", "--help"], std::path::Path::new("/tmp")).unwrap(),
            CommandLine::Help
        );
        for args in [
            vec!["optionterm", "-e"],
            vec!["optionterm", "-d"],
            vec!["optionterm", "--unknown"],
        ] {
            assert!(parse_command_line(&args, std::path::Path::new("/tmp")).is_err());
        }
    }

    #[test]
    fn relative_paths_are_resolved_against_the_caller() {
        let dir = crate::test_support::TestDir::new("cli-relative");
        std::fs::create_dir(dir.path().join("project")).unwrap();
        let CommandLine::Launch(request) =
            parse_command_line(&["optionterm", "-d", "project"], dir.path()).unwrap()
        else {
            panic!("expected launch")
        };
        assert_eq!(request.cwd.unwrap(), dir.path().join("project"));
    }

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
