//! The broker's configuration, read from its environment before anything starts (design 3.3,
//! lesson DEF-2 of the demo).
//!
//! The host starts the broker with an empty environment and no arguments, then adds the data
//! directory and the key source, and in a synthetic test run the synthetic agent (the three
//! variables below). The broker reads those three, refuses any other name that starts with
//! `FABRIC_BROKER_` (a misspelt name is an error, not a silent default), and ignores every
//! variable outside that prefix. A refusal is one stderr line naming the variable and the reason,
//! and exit code 78; it never prints the value of a variable other than a path.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

/// The exit code after a failed write to the store: the run is over (`EX_TEMPFAIL`).
pub const EXIT_HALT: u8 = 75;
/// The exit code for a refused configuration (`EX_CONFIG`).
pub const EXIT_CONFIG: u8 = 78;
/// Every variable of the broker starts with this.
pub const VARIABLE_PREFIX: &str = "FABRIC_BROKER_";
/// The absolute path of the data directory.
pub const DATA_DIR: &str = "FABRIC_BROKER_DATA_DIR";
/// Where the store key comes from: `keychain` (macOS) or `test` (debug builds).
pub const KEY_SOURCE: &str = "FABRIC_BROKER_KEY_SOURCE";
/// `1` starts the synthetic agent (debug builds, test key only).
pub const SYNTHETIC_AGENT: &str = "FABRIC_BROKER_SYNTHETIC_AGENT";

/// Where the store key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// The generic password in the login Keychain.
    #[cfg(target_os = "macos")]
    Keychain,
    /// The public test key. Debug builds only.
    #[cfg(debug_assertions)]
    Test,
}

/// A checked configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// The data directory, absolute; the store creates it when it is missing.
    pub data_dir: PathBuf,
    /// The source of the store key.
    pub key_source: KeySource,
    /// True when the synthetic agent runs (debug builds, test key only).
    pub synthetic_agent: bool,
}

/// One refusal: the variable and the reason, as one stderr line.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// True when `name` starts with [`VARIABLE_PREFIX`]. Windows compares names without regard to
/// case, as Windows itself does.
#[must_use]
pub fn is_broker_variable(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    let prefix = VARIABLE_PREFIX.as_bytes();
    bytes.len() >= prefix.len()
        && if cfg!(windows) {
            bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
        } else {
            &bytes[..prefix.len()] == prefix
        }
}

/// Whether `name` is the variable `wanted`, by the platform's rule.
fn is(name: &OsStr, wanted: &str) -> bool {
    if cfg!(windows) {
        name.as_encoded_bytes()
            .eq_ignore_ascii_case(wanted.as_bytes())
    } else {
        name == OsStr::new(wanted)
    }
}

fn refuse(variable: &str, reason: &str) -> ConfigError {
    ConfigError(format!("fabric-broker: {variable} {reason}"))
}

/// Reads the three names of 3.3; refuses any other `FABRIC_BROKER_*` name, and the synthetic agent without the
/// test key; ignores every variable outside the prefix.
///
/// The data directory must be absolute and, when something exists at the path, a directory.
/// Whether a missing directory can be created, and the mode of an existing one, are checked by
/// the store when it opens, before any frame (exit 78 too).
///
/// # Errors
///
/// A [`ConfigError`] naming the first variable refused.
pub fn config_from(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Config, ConfigError> {
    let mut data_dir = None;
    let mut key_source = None;
    let mut synthetic = None;
    for (name, value) in vars {
        if !is_broker_variable(&name) {
            continue;
        }
        if is(&name, DATA_DIR) {
            data_dir = Some(value);
        } else if is(&name, KEY_SOURCE) {
            key_source = Some(value);
        } else if is(&name, SYNTHETIC_AGENT) {
            synthetic = Some(value);
        } else {
            return Err(ConfigError(format!(
                "fabric-broker: {} is not a variable of the broker; it reads only {DATA_DIR}, {KEY_SOURCE} and {SYNTHETIC_AGENT}",
                name.to_string_lossy()
            )));
        }
    }
    let data_dir = data_dir_from(data_dir)?;
    let key_source = key_source_from(key_source)?;
    let synthetic_agent = synthetic_from(synthetic, key_source)?;
    Ok(Config {
        data_dir,
        key_source,
        synthetic_agent,
    })
}

fn data_dir_from(value: Option<OsString>) -> Result<PathBuf, ConfigError> {
    let Some(value) = value else {
        return Err(refuse(
            DATA_DIR,
            "is not set; it must name the broker's data directory by an absolute path",
        ));
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(refuse(
            DATA_DIR,
            &format!("is {:?}, which is not an absolute path", path.display()),
        ));
    }
    match std::fs::metadata(&path) {
        Ok(metadata) if !metadata.is_dir() => Err(refuse(
            DATA_DIR,
            &format!("is {:?}, which is not a directory", path.display()),
        )),
        _ => Ok(path),
    }
}

fn key_source_from(value: Option<OsString>) -> Result<KeySource, ConfigError> {
    let Some(value) = value else {
        return Err(refuse(
            KEY_SOURCE,
            "is not set; it must be keychain (macOS) or test (debug builds)",
        ));
    };
    if value == "keychain" {
        #[cfg(target_os = "macos")]
        return Ok(KeySource::Keychain);
        #[cfg(not(target_os = "macos"))]
        return Err(refuse(
            KEY_SOURCE,
            "is keychain, which exists only on macOS; this platform has the test key in debug builds only",
        ));
    }
    if value == "test" {
        #[cfg(debug_assertions)]
        return Ok(KeySource::Test);
        #[cfg(not(debug_assertions))]
        return Err(refuse(
            KEY_SOURCE,
            "is test, which a release build refuses; it opens stores with the Keychain only",
        ));
    }
    Err(refuse(
        KEY_SOURCE,
        "has a value the broker does not know; it must be keychain (macOS) or test (debug builds)",
    ))
}

fn synthetic_from(value: Option<OsString>, key_source: KeySource) -> Result<bool, ConfigError> {
    let Some(value) = value else {
        return Ok(false);
    };
    if value != "1" {
        return Err(refuse(
            SYNTHETIC_AGENT,
            "has a value the broker does not know; it must be 1 or absent",
        ));
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = key_source;
        Err(refuse(
            SYNTHETIC_AGENT,
            "is set, which a release build refuses; it has no synthetic agent",
        ))
    }
    #[cfg(debug_assertions)]
    match key_source {
        KeySource::Test => Ok(true),
        #[cfg(target_os = "macos")]
        KeySource::Keychain => Err(refuse(
            SYNTHETIC_AGENT,
            &format!(
                "is 1, which needs {KEY_SOURCE}=test: synthetic records never go into a Keychain store"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    //! These tests only read the variables they are given; none of them opens a store or
    //! touches the Keychain.

    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    fn absolute() -> String {
        std::env::temp_dir()
            .join("fabric-broker-config-test-no-such-dir")
            .to_string_lossy()
            .into_owned()
    }

    fn refused(list: &[(&str, &str)]) -> String {
        match config_from(vars(list)) {
            Err(ConfigError(line)) => line,
            Ok(config) => panic!("accepted: {config:?}"),
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_test_key_and_a_directory_are_a_configuration() {
        let dir = absolute();
        let config = config_from(vars(&[
            ("HOME", "/elsewhere"),
            (DATA_DIR, &dir),
            (KEY_SOURCE, "test"),
        ]))
        .unwrap();
        assert_eq!(config.data_dir, PathBuf::from(&dir));
        assert_eq!(config.key_source, KeySource::Test);
        assert!(!config.synthetic_agent);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_synthetic_agent_runs_with_the_test_key() {
        let dir = absolute();
        let config = config_from(vars(&[
            (DATA_DIR, &dir),
            (KEY_SOURCE, "test"),
            (SYNTHETIC_AGENT, "1"),
        ]))
        .unwrap();
        assert!(config.synthetic_agent);
    }

    #[test]
    fn the_synthetic_agent_is_refused_without_the_test_key() {
        let dir = absolute();
        let line = refused(&[
            (DATA_DIR, &dir),
            (KEY_SOURCE, "keychain"),
            (SYNTHETIC_AGENT, "1"),
        ]);
        if cfg!(target_os = "macos") {
            // The Keychain is a key source on macOS in both profiles, so the synthetic agent is
            // what is refused: in a debug build for want of the test key, in a release build
            // because it has none.
            assert!(line.contains(SYNTHETIC_AGENT), "{line}");
        } else {
            // Elsewhere the key source itself is refused first.
            assert!(line.contains(KEY_SOURCE), "{line}");
        }
    }

    #[test]
    fn a_missing_variable_is_named() {
        let dir = absolute();
        assert!(refused(&[(KEY_SOURCE, "test")]).contains(DATA_DIR));
        assert!(refused(&[(DATA_DIR, &dir)]).contains(KEY_SOURCE));
    }

    #[test]
    fn a_relative_data_directory_is_refused() {
        let line = refused(&[(DATA_DIR, "data"), (KEY_SOURCE, "test")]);
        assert!(
            line.contains(DATA_DIR) && line.contains("absolute"),
            "{line}"
        );
    }

    #[test]
    fn a_data_directory_that_is_a_file_is_refused() {
        let file = std::env::current_exe().unwrap();
        let line = refused(&[(DATA_DIR, &file.to_string_lossy()), (KEY_SOURCE, "test")]);
        assert!(
            line.contains(DATA_DIR) && line.contains("not a directory"),
            "{line}"
        );
    }

    #[test]
    fn an_unknown_value_is_refused_without_printing_it() {
        let dir = absolute();
        let line = refused(&[(DATA_DIR, &dir), (KEY_SOURCE, "secret-value-7f3a")]);
        assert!(line.contains(KEY_SOURCE), "{line}");
        assert!(!line.contains("secret-value-7f3a"), "{line}");
        let line = refused(&[
            (DATA_DIR, &dir),
            (KEY_SOURCE, "test"),
            (SYNTHETIC_AGENT, "secret-value-7f3a"),
        ]);
        // A release build refuses the test key before it reads the synthetic agent.
        let named = if cfg!(debug_assertions) {
            SYNTHETIC_AGENT
        } else {
            KEY_SOURCE
        };
        assert!(line.contains(named), "{line}");
        assert!(!line.contains("secret-value-7f3a"), "{line}");
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_refuses_the_test_key() {
        let dir = absolute();
        let line = refused(&[(DATA_DIR, &dir), (KEY_SOURCE, "test")]);
        assert!(
            line.contains(KEY_SOURCE) && line.contains("release build"),
            "{line}"
        );
    }

    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    #[test]
    fn a_release_build_refuses_the_synthetic_agent() {
        // Only the variables are read; nothing here opens a store.
        let dir = absolute();
        let line = refused(&[
            (DATA_DIR, &dir),
            (KEY_SOURCE, "keychain"),
            (SYNTHETIC_AGENT, "1"),
        ]);
        assert!(
            line.contains(SYNTHETIC_AGENT) && line.contains("release build"),
            "{line}"
        );
    }

    #[test]
    fn the_variable_names_are_those_the_host_and_the_relay_spell() {
        // The relay's allowlist and the host spell these names too (fabric-desktop-relay); the
        // relay's test `allowlist_names_the_broker_variables` pins the same literals.
        assert_eq!(VARIABLE_PREFIX, "FABRIC_BROKER_");
        assert_eq!(DATA_DIR, "FABRIC_BROKER_DATA_DIR");
        assert_eq!(KEY_SOURCE, "FABRIC_BROKER_KEY_SOURCE");
        assert_eq!(SYNTHETIC_AGENT, "FABRIC_BROKER_SYNTHETIC_AGENT");
    }

    #[test]
    fn an_unknown_broker_variable_is_refused() {
        let dir = absolute();
        let line = refused(&[
            (DATA_DIR, &dir),
            (KEY_SOURCE, "test"),
            ("FABRIC_BROKER_KEYSOURCE", "test"),
        ]);
        assert!(line.contains("FABRIC_BROKER_KEYSOURCE"), "{line}");
    }

    #[test]
    fn variables_outside_the_prefix_are_ignored() {
        assert!(!is_broker_variable(OsStr::new("FABRIC_DESKTOP_DATA_DIR")));
        assert!(!is_broker_variable(OsStr::new("FABRIC_BROKER")));
        assert!(is_broker_variable(OsStr::new("FABRIC_BROKER_UNDER_TEST")));
        assert_eq!(
            is_broker_variable(OsStr::new("fabric_broker_x")),
            cfg!(windows)
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_keychain_is_refused_off_macos() {
        let dir = absolute();
        let line = refused(&[(DATA_DIR, &dir), (KEY_SOURCE, "keychain")]);
        assert!(
            line.contains(KEY_SOURCE) && line.contains("macOS"),
            "{line}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_keychain_is_a_key_source_on_macos() {
        // Only the variables are read; nothing here opens a store.
        let dir = absolute();
        let config = config_from(vars(&[(DATA_DIR, &dir), (KEY_SOURCE, "keychain")])).unwrap();
        assert_eq!(config.key_source, KeySource::Keychain);
    }
}
