//! The command line (W06 2.2).
//!
//! ```text
//! fabric-mcp-bridge --enrollment <id> [--socket <path> --key-dir <dir>]
//! fabric-mcp-bridge code --enrollment <id> [--key-dir <dir>]
//! fabric-mcp-bridge config --client <claude-desktop|claude-code|codex> --enrollment <id>
//!                   [--socket <path> --key-dir <dir>]
//! fabric-mcp-bridge --version
//! ```
//!
//! `--socket` and `--key-dir` exist only in debug builds (A23); whether this is one is a
//! parameter of [`parse`], so both behaviours are tested in one run. A wrong command line is
//! exit 64 before anything is read from stdin.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use fabric_desktop_contracts::scalar::Id;

use crate::config_text::Client;

/// The exit code for a wrong command line (`EX_USAGE`).
pub const EXIT_USAGE: u8 = 64;

/// The two debug seams, which come together or not at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seams {
    pub socket: PathBuf,
    pub key_dir: PathBuf,
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Serve MCP on stdin and stdout.
    Serve {
        enrollment: Id,
        seams: Option<Seams>,
    },
    /// Print the pairing code.
    Code {
        enrollment: Id,
        key_dir: Option<PathBuf>,
    },
    /// Print the configuration text.
    Config {
        client: Client,
        enrollment: Id,
        seams: Option<Seams>,
    },
    /// Print the version.
    Version,
}

/// Why the command line is wrong. The messages are fixed and never repeat an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliError {
    NotUtf8,
    Unknown,
    Repeated,
    MissingValue,
    MissingEnrollment,
    BadEnrollment,
    MissingClient,
    BadClient,
    DebugOnly,
    NotAbsolute,
    SeamsApart,
}

impl CliError {
    /// Every wrong command line is exit 64.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        EXIT_USAGE
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CliError::NotUtf8 => "an argument is not UTF-8",
            CliError::Unknown => "an argument is not known",
            CliError::Repeated => "an option is given twice",
            CliError::MissingValue => "an option has no value",
            CliError::MissingEnrollment => "--enrollment is missing",
            CliError::BadEnrollment => "the enrollment id is not an id",
            CliError::MissingClient => "--client is missing",
            CliError::BadClient => "--client is not claude-desktop, claude-code or codex",
            CliError::DebugOnly => "--socket and --key-dir exist only in debug builds",
            CliError::NotAbsolute => "--socket and --key-dir take absolute paths",
            CliError::SeamsApart => "--socket and --key-dir come together or not at all",
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for CliError {}

/// The one-line usage, for stderr after an error.
pub const USAGE: &str = "usage: fabric-mcp-bridge --enrollment <id> | code --enrollment <id> | config --client <claude-desktop|claude-code|codex> --enrollment <id> | --version";

#[derive(Default)]
struct Options {
    enrollment: Option<String>,
    socket: Option<String>,
    key_dir: Option<String>,
    client: Option<String>,
}

/// Parses the arguments after the program's name.
pub fn parse<I>(args: I, debug_build: bool) -> Result<Command, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.into_string().map_err(|_| CliError::NotUtf8))
        .collect::<Result<_, _>>()?;
    let (mode, rest) = match args.first().map(String::as_str) {
        Some("--version") => {
            return if args.len() == 1 {
                Ok(Command::Version)
            } else {
                Err(CliError::Unknown)
            };
        }
        Some("code") => ("code", &args[1..]),
        Some("config") => ("config", &args[1..]),
        _ => ("serve", &args[..]),
    };
    let options = read_options(rest, mode == "config")?;
    if !debug_build && (options.socket.is_some() || options.key_dir.is_some()) {
        return Err(CliError::DebugOnly);
    }
    let enrollment = options
        .enrollment
        .ok_or(CliError::MissingEnrollment)
        .and_then(|e| Id::new(e).map_err(|_| CliError::BadEnrollment))?;
    let socket = options.socket.map(absolute).transpose()?;
    let key_dir = options.key_dir.map(absolute).transpose()?;
    match mode {
        "code" => {
            if socket.is_some() {
                return Err(CliError::Unknown);
            }
            Ok(Command::Code {
                enrollment,
                key_dir,
            })
        }
        "config" => {
            let client = options
                .client
                .ok_or(CliError::MissingClient)
                .and_then(|c| Client::from_name(&c).ok_or(CliError::BadClient))?;
            Ok(Command::Config {
                client,
                enrollment,
                seams: seams(socket, key_dir)?,
            })
        }
        _ => Ok(Command::Serve {
            enrollment,
            seams: seams(socket, key_dir)?,
        }),
    }
}

fn read_options(args: &[String], with_client: bool) -> Result<Options, CliError> {
    let mut options = Options::default();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let slot = match flag.as_str() {
            "--enrollment" => &mut options.enrollment,
            "--socket" => &mut options.socket,
            "--key-dir" => &mut options.key_dir,
            "--client" if with_client => &mut options.client,
            _ => return Err(CliError::Unknown),
        };
        if slot.is_some() {
            return Err(CliError::Repeated);
        }
        *slot = Some(iter.next().ok_or(CliError::MissingValue)?.clone());
    }
    Ok(options)
}

fn absolute(path: String) -> Result<PathBuf, CliError> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(CliError::NotAbsolute)
    }
}

fn seams(socket: Option<PathBuf>, key_dir: Option<PathBuf>) -> Result<Option<Seams>, CliError> {
    match (socket, key_dir) {
        (Some(socket), Some(key_dir)) => Ok(Some(Seams { socket, key_dir })),
        (None, None) => Ok(None),
        _ => Err(CliError::SeamsApart),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &[&str]) -> Vec<OsString> {
        line.iter().map(OsString::from).collect()
    }

    fn abs(p: &str) -> String {
        // An absolute path on every system.
        std::env::temp_dir().join(p).to_string_lossy().into_owned()
    }

    // a01: a wrong command line is exit 64; a release build refuses --socket and --key-dir.
    #[test]
    fn a01_command_line() {
        let s = abs("s/agent.sock");
        let k = abs("k");
        for debug in [true, false] {
            let wrong: Vec<Vec<&str>> = vec![
                vec![],
                vec!["--enrollment"],
                vec!["--enrollment", "-bad"],
                vec!["--enrollment", "a", "--enrollment", "b"],
                vec!["--enrollment", "a", "--verbose"],
                vec!["--enrollment", "a", "extra"],
                vec!["--version", "--enrollment", "a"],
                vec!["code"],
                vec!["code", "--enrollment", "a", "--client", "codex"],
                vec!["config", "--enrollment", "a"],
                vec!["config", "--client", "vim", "--enrollment", "a"],
                vec!["--enrollment", "a", "--client", "codex"],
                vec![
                    "--enrollment",
                    "a",
                    "--socket",
                    "relative/agent.sock",
                    "--key-dir",
                    &k,
                ],
                vec!["--enrollment", "a", "--socket", &s],
                vec!["--enrollment", "a", "--key-dir", &k],
                vec!["code", "--enrollment", "a", "--socket", &s],
            ];
            for line in wrong {
                let e = parse(args(&line), debug).unwrap_err();
                assert_eq!(e.exit_code(), 64, "{line:?} {debug}");
            }
            assert_eq!(parse(args(&["--version"]), debug), Ok(Command::Version));
            assert_eq!(
                parse(args(&["--enrollment", "agent-1"]), debug),
                Ok(Command::Serve {
                    enrollment: Id::new("agent-1").unwrap(),
                    seams: None
                })
            );
            assert_eq!(
                parse(args(&["code", "--enrollment", "agent-1"]), debug),
                Ok(Command::Code {
                    enrollment: Id::new("agent-1").unwrap(),
                    key_dir: None
                })
            );
            assert_eq!(
                parse(
                    args(&["config", "--client", "codex", "--enrollment", "agent-1"]),
                    debug
                ),
                Ok(Command::Config {
                    client: Client::Codex,
                    enrollment: Id::new("agent-1").unwrap(),
                    seams: None
                })
            );
        }
        let with_seams = ["--enrollment", "agent-1", "--socket", &s, "--key-dir", &k];
        assert_eq!(
            parse(args(&with_seams), true),
            Ok(Command::Serve {
                enrollment: Id::new("agent-1").unwrap(),
                seams: Some(Seams {
                    socket: PathBuf::from(&s),
                    key_dir: PathBuf::from(&k)
                })
            })
        );
        assert_eq!(parse(args(&with_seams), false), Err(CliError::DebugOnly));
        let code = ["code", "--enrollment", "agent-1", "--key-dir", &k];
        assert!(parse(args(&code), true).is_ok());
        assert_eq!(parse(args(&code), false), Err(CliError::DebugOnly));
        let config = [
            "config",
            "--client",
            "claude-code",
            "--enrollment",
            "agent-1",
            "--socket",
            &s,
            "--key-dir",
            &k,
        ];
        assert!(parse(args(&config), true).is_ok());
        assert_eq!(parse(args(&config), false), Err(CliError::DebugOnly));
        // Ids that are not contract ids are a wrong command line in every form, before any
        // file is touched: parsing reads no file, and the key directory named is not made.
        let untouched = abs(&format!(
            "fmb-a01-{}/keys",
            crate::hex(&crate::random_128().unwrap()[..4])
        ));
        for bad in ["..", "a/b", ".hidden", ""] {
            let forms: [Vec<&str>; 3] = [
                vec!["--enrollment", bad],
                vec!["code", "--enrollment", bad, "--key-dir", &untouched],
                vec!["config", "--client", "codex", "--enrollment", bad],
            ];
            for form in forms {
                let e = parse(args(&form), true).unwrap_err();
                assert_eq!(e, CliError::BadEnrollment, "{form:?}");
                assert_eq!(e.exit_code(), 64);
            }
        }
        assert!(!std::path::Path::new(&untouched).exists());
        // The messages never repeat an argument.
        assert!(!CliError::BadEnrollment.as_str().contains("-bad"));
    }
}
