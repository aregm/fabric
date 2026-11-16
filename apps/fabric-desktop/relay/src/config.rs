//! The broker's configuration, checked before any process starts.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The names of the host's environment variables that the broker child receives. Every other
/// variable of the host is left out.
///
/// It is empty, except on Windows, where a process started with no `SystemRoot` can fail to
/// start. The spike broker reads no variable; a later work package that needs one adds its name
/// here with the reason.
#[cfg(windows)]
pub const ENVIRONMENT_ALLOWLIST: &[&str] = &["SystemRoot"];

/// The names of the host's environment variables that the broker child receives. Every other
/// variable of the host is left out.
///
/// It is empty, except on Windows, where a process started with no `SystemRoot` can fail to
/// start. The spike broker reads no variable; a later work package that needs one adds its name
/// here with the reason.
#[cfg(not(windows))]
pub const ENVIRONMENT_ALLOWLIST: &[&str] = &[];

/// The broker executable that ships beside the host executable: `fabric-broker` (with `.exe` on
/// Windows) in the host's directory.
#[must_use]
pub fn broker_beside(host_executable: &Path) -> PathBuf {
    let name = format!("fabric-broker{}", std::env::consts::EXE_SUFFIX);
    match host_executable.parent() {
        Some(directory) => directory.join(name),
        None => PathBuf::from(name),
    }
}

/// Why a broker configuration was refused. Each carries the path that was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The path is relative; the broker is always started by an absolute path.
    NotAbsolute(PathBuf),
    /// Nothing can be read at the path (it does not exist, or its metadata cannot be read).
    Missing(PathBuf),
    /// The path names a directory or something else that is not a file.
    NotAFile(PathBuf),
    /// The file has no execute permission bit (Unix only).
    NotExecutable(PathBuf),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotAbsolute(p) => {
                write!(f, "the broker path {} is not absolute", p.display())
            }
            ConfigError::Missing(p) => write!(f, "there is no broker at {}", p.display()),
            ConfigError::NotAFile(p) => write!(f, "the broker path {} is not a file", p.display()),
            ConfigError::NotExecutable(p) => {
                write!(f, "the broker at {} is not executable", p.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// A checked broker configuration: an absolute path to an executable file and the environment
/// the child receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerConfig {
    program: PathBuf,
    environment: Vec<(OsString, OsString)>,
}

impl BrokerConfig {
    /// Everything that can be checked before a process starts (lesson DEF-2 of the demo).
    ///
    /// Refuses a relative path, a path where nothing can be read, a path that is not a file,
    /// and on Unix a file with no execute bit. Keeps the variables of `host_environment` whose
    /// names are in `allowlist`. Starts nothing.
    pub fn check(
        program: &Path,
        allowlist: &[&str],
        host_environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<BrokerConfig, ConfigError> {
        check_program(program)?;
        Ok(BrokerConfig {
            program: program.to_path_buf(),
            environment: filter_environment(allowlist, host_environment),
        })
    }

    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The variables the child receives, in the order of the allowlist.
    #[must_use]
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// `env_clear`, then exactly `environment`; stdin and stdout piped; stderr inherited; no
    /// arguments.
    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .envs(self.environment.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command
    }
}

fn check_program(program: &Path) -> Result<(), ConfigError> {
    if !program.is_absolute() {
        return Err(ConfigError::NotAbsolute(program.to_path_buf()));
    }
    // `metadata` follows symbolic links: what counts is the file that would be executed.
    let metadata =
        std::fs::metadata(program).map_err(|_| ConfigError::Missing(program.to_path_buf()))?;
    if !metadata.is_file() {
        return Err(ConfigError::NotAFile(program.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ConfigError::NotExecutable(program.to_path_buf()));
        }
    }
    Ok(())
}

/// Whether an environment variable's name is the allowlisted `name`. Windows compares names
/// without regard to case, as Windows itself does.
fn same_name(variable: &OsStr, name: &str) -> bool {
    if cfg!(windows) {
        variable
            .to_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(name))
    } else {
        variable == OsStr::new(name)
    }
}

/// The variables of `host_environment` whose names are in `allowlist`, in the order of the
/// allowlist. When the host lists a name twice, the last value counts, as it would in a process
/// environment that was set twice.
fn filter_environment(
    allowlist: &[&str],
    host_environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut found: Vec<Option<(OsString, OsString)>> = vec![None; allowlist.len()];
    for (variable, value) in host_environment {
        if let Some(index) = allowlist.iter().position(|n| same_name(&variable, n)) {
            found[index] = Some((variable, value));
        }
    }
    found.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn allowlist_is_empty_except_on_windows() {
        if cfg!(windows) {
            assert_eq!(ENVIRONMENT_ALLOWLIST, &["SystemRoot"]);
        } else {
            assert!(ENVIRONMENT_ALLOWLIST.is_empty());
        }
    }

    #[test]
    fn filter_keeps_only_allowlisted_names_in_allowlist_order() {
        let host = pairs(&[("HOME", "/h"), ("B", "2"), ("SECRET", "s"), ("A", "1")]);
        let kept = filter_environment(&["A", "B", "MISSING"], host);
        assert_eq!(kept, pairs(&[("A", "1"), ("B", "2")]));
    }

    #[test]
    fn filter_with_empty_allowlist_keeps_nothing() {
        let host = pairs(&[("PATH", "/bin"), ("HOME", "/h")]);
        assert!(filter_environment(&[], host).is_empty());
    }

    #[test]
    fn filter_takes_the_last_value_of_a_repeated_name() {
        let host = pairs(&[("A", "1"), ("A", "2")]);
        assert_eq!(filter_environment(&["A"], host), pairs(&[("A", "2")]));
    }

    #[test]
    fn filter_compares_names_by_the_platform_rule() {
        let host = pairs(&[("systemroot", "C:\\Windows")]);
        let kept = filter_environment(&["SystemRoot"], host);
        if cfg!(windows) {
            assert_eq!(kept.len(), 1);
        } else {
            assert!(kept.is_empty());
        }
    }

    #[test]
    fn broker_beside_names_the_file_in_the_host_directory() {
        let host = Path::new("/Applications/Fabric.app/Contents/MacOS/fabric-desktop");
        let expected = format!(
            "/Applications/Fabric.app/Contents/MacOS/fabric-broker{}",
            std::env::consts::EXE_SUFFIX
        );
        assert_eq!(broker_beside(host), PathBuf::from(expected));
    }

    #[test]
    fn broker_beside_keeps_spaces() {
        let host = Path::new("/a dir/with space/host");
        assert_eq!(
            broker_beside(host).parent(),
            Some(Path::new("/a dir/with space"))
        );
    }

    #[test]
    fn check_refuses_a_relative_path() {
        let path = Path::new("fabric-broker");
        assert_eq!(
            BrokerConfig::check(path, &[], Vec::new()),
            Err(ConfigError::NotAbsolute(path.to_path_buf()))
        );
    }

    #[test]
    fn check_refuses_a_missing_path() {
        let path = std::env::temp_dir().join("fabric-desktop-relay-no-such-file-6b1f");
        assert_eq!(
            BrokerConfig::check(&path, &[], Vec::new()),
            Err(ConfigError::Missing(path))
        );
    }

    #[test]
    fn check_refuses_a_directory() {
        let path = std::env::temp_dir();
        assert_eq!(
            BrokerConfig::check(&path, &[], Vec::new()),
            Err(ConfigError::NotAFile(path))
        );
    }

    #[test]
    fn check_accepts_the_running_executable() {
        let path = std::env::current_exe().unwrap();
        let config = BrokerConfig::check(&path, &["A"], pairs(&[("A", "1"), ("B", "2")])).unwrap();
        assert_eq!(config.program(), path);
        assert_eq!(config.environment(), pairs(&[("A", "1")]).as_slice());
        let command = config.command();
        assert_eq!(command.get_program(), path.as_os_str());
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn errors_name_the_path() {
        let text = ConfigError::NotExecutable(PathBuf::from("/x/y")).to_string();
        assert_eq!(text, "the broker at /x/y is not executable");
    }
}
