//! The broker's configuration, read from its environment before anything starts (design 3.3,
//! lesson DEF-2 of the demo).
//!
//! The host starts the broker with no arguments and an environment cleared of everything but,
//! on Unix, the relay's fixed `PATH`, then adds the data directory and the key source, in a
//! synthetic test run the synthetic agent, and where a rule prover runs the three Isabelle paths
//! (the six variables below). The broker reads those six,
//! refuses any other name that starts with `FABRIC_BROKER_` (a misspelt name is an error, not a
//! silent default), and ignores every variable outside that prefix. A refusal is one stderr line
//! naming the variable and the reason, and exit code 78; it never prints the value of a variable
//! other than a path.
//!
//! The three Isabelle paths are set all together or not at all. Each is checked here with no more
//! than a look at the file system; whether Isabelle starts, and whether the rule theory is the one
//! the broker was built with, the prover finds out after the store is open.
//!
//! Bare numbers here, such as (3.3) or (D27), cite the CD-W04 design
//! (`docs/plans/calendar-desktop-w04-design.md`); "PW" numbers, such as the test ids PW c40 to
//! PW c43, cite the prover wiring's design (`docs/plans/calendar-desktop-prover-wiring-design.md`).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

/// The exit code when stdin or stdout fails (`EX_IOERR`); moved here from main.
pub const EXIT_IO: u8 = 74;
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
/// The Isabelle distribution root; `<it>/bin/isabelle` is run.
pub const ISABELLE_HOME: &str = "FABRIC_BROKER_ISABELLE_HOME";
/// Isabelle's per-user directory (`USER_HOME`): settings, heaps, server registry.
pub const ISABELLE_USER_HOME: &str = "FABRIC_BROKER_ISABELLE_USER_HOME";
/// The directory with `ROOTS` and `Fabric_Broker/`.
pub const RULES_DIR: &str = "FABRIC_BROKER_RULES_DIR";

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

/// Where Isabelle and the rule theory are: all three variables, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsabelleSettings {
    /// The value of [`ISABELLE_HOME`], absolute.
    pub isabelle_home: PathBuf,
    /// The value of [`ISABELLE_USER_HOME`], absolute.
    pub user_home: PathBuf,
    /// The value of [`RULES_DIR`], absolute.
    pub rules_dir: PathBuf,
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
    /// `Some` when the three Isabelle variables are set (all or none).
    pub isabelle: Option<IsabelleSettings>,
}

impl Config {
    /// True when this run uses the practice calendar: a debug build, the synthetic agent and a
    /// configured Isabelle.
    #[must_use]
    pub fn practice(&self) -> bool {
        cfg!(debug_assertions) && self.synthetic_agent && self.isabelle.is_some()
    }
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

/// Reads the six names (3.3); refuses any other `FABRIC_BROKER_*` name, the
/// synthetic agent without the test key, and one or two of the Isabelle paths without the
/// others; ignores every variable outside the prefix.
///
/// The data directory must be absolute and, when something exists at the path, a directory.
/// Whether a missing directory can be created, and the mode of an existing one, are checked by
/// the store when it opens, before any frame (exit 78 too). The Isabelle paths must be absolute
/// and lead to what Isabelle and the rule theory need.
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
    let mut isabelle_home = None;
    let mut user_home = None;
    let mut rules_dir = None;
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
        } else if is(&name, ISABELLE_HOME) {
            isabelle_home = Some(value);
        } else if is(&name, ISABELLE_USER_HOME) {
            user_home = Some(value);
        } else if is(&name, RULES_DIR) {
            rules_dir = Some(value);
        } else {
            return Err(ConfigError(format!(
                "fabric-broker: {} is not a variable of the broker; it reads only {DATA_DIR}, {KEY_SOURCE}, {SYNTHETIC_AGENT}, {ISABELLE_HOME}, {ISABELLE_USER_HOME} and {RULES_DIR}",
                name.to_string_lossy()
            )));
        }
    }
    let data_dir = data_dir_from(data_dir)?;
    let key_source = key_source_from(key_source)?;
    let synthetic_agent = synthetic_from(synthetic, key_source)?;
    let isabelle = isabelle_from(isabelle_home, user_home, rules_dir)?;
    Ok(Config {
        data_dir,
        key_source,
        synthetic_agent,
        isabelle,
    })
}

/// The refusal of one or two Isabelle paths without the others.
const NOT_TOGETHER: &str = "fabric-broker: the rule prover needs FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR together, or none";

/// The Isabelle paths: none, or all three, each checked.
///
/// - [`ISABELLE_HOME`] must be absolute and hold `bin/isabelle`, a file (followed through a
///   symbolic link, as the `.app` layout needs) that, on Unix, may be executed.
/// - [`ISABELLE_USER_HOME`] must be absolute and an existing directory; the broker never creates
///   it.
/// - [`RULES_DIR`] must be absolute and hold the files `ROOTS`, `Fabric_Broker/ROOT` and
///   `Fabric_Broker/Fabric_Broker.thy`.
///
/// Nothing heavier than a look at the file's metadata runs here.
///
/// # Errors
///
/// A [`ConfigError`] naming the first variable refused, or all three when only some are set.
fn isabelle_from(
    isabelle_home: Option<OsString>,
    user_home: Option<OsString>,
    rules_dir: Option<OsString>,
) -> Result<Option<IsabelleSettings>, ConfigError> {
    let (isabelle_home, user_home, rules_dir) = match (isabelle_home, user_home, rules_dir) {
        (None, None, None) => return Ok(None),
        (Some(home), Some(user), Some(rules)) => (home, user, rules),
        _ => return Err(ConfigError(NOT_TOGETHER.to_owned())),
    };
    let isabelle_home = absolute(ISABELLE_HOME, isabelle_home)?;
    let program = isabelle_home.join("bin").join("isabelle");
    match std::fs::metadata(&program) {
        Ok(metadata) if metadata.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return Err(refuse(
                        ISABELLE_HOME,
                        &format!(
                            "is {:?}, whose bin/isabelle may not be executed",
                            isabelle_home.display()
                        ),
                    ));
                }
            }
        }
        _ => {
            return Err(refuse(
                ISABELLE_HOME,
                &format!(
                    "is {:?}, which holds no file bin/isabelle",
                    isabelle_home.display()
                ),
            ));
        }
    }
    let user_home = absolute(ISABELLE_USER_HOME, user_home)?;
    if !std::fs::metadata(&user_home).is_ok_and(|m| m.is_dir()) {
        return Err(refuse(
            ISABELLE_USER_HOME,
            &format!(
                "is {:?}, which is not an existing directory; the broker does not create it",
                user_home.display()
            ),
        ));
    }
    let rules_dir = absolute(RULES_DIR, rules_dir)?;
    for file in [
        "ROOTS",
        "Fabric_Broker/ROOT",
        "Fabric_Broker/Fabric_Broker.thy",
    ] {
        if !std::fs::metadata(rules_dir.join(file)).is_ok_and(|m| m.is_file()) {
            return Err(refuse(
                RULES_DIR,
                &format!("is {:?}, which holds no file {file}", rules_dir.display()),
            ));
        }
    }
    Ok(Some(IsabelleSettings {
        isabelle_home,
        user_home,
        rules_dir,
    }))
}

/// The path of `variable`, which must be absolute.
fn absolute(variable: &str, value: OsString) -> Result<PathBuf, ConfigError> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(refuse(
            variable,
            &format!("is {:?}, which is not an absolute path", path.display()),
        ))
    }
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
    #[cfg(any(debug_assertions, target_os = "macos"))]
    use std::path::Path;

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

    /// A layout that passes every check of the three Isabelle paths, made of empty files: no
    /// Isabelle runs from it. Removed on drop.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    struct Layout {
        root: PathBuf,
        home: PathBuf,
        user: PathBuf,
        rules: PathBuf,
    }

    #[cfg(any(debug_assertions, target_os = "macos"))]
    impl Layout {
        fn new(label: &str) -> Layout {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "fabric-broker-config-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let home = root.join("isabelle");
            let user = root.join("user");
            let rules = root.join("rules");
            std::fs::create_dir_all(home.join("bin")).unwrap();
            std::fs::create_dir_all(&user).unwrap();
            std::fs::create_dir_all(rules.join("Fabric_Broker")).unwrap();
            let program = home.join("bin").join("isabelle");
            std::fs::write(&program, b"#!/bin/sh\nexit 1\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            for file in [
                "ROOTS",
                "Fabric_Broker/ROOT",
                "Fabric_Broker/Fabric_Broker.thy",
            ] {
                std::fs::write(rules.join(file), b"").unwrap();
            }
            Layout {
                root,
                home,
                user,
                rules,
            }
        }

        fn text(path: &Path) -> String {
            path.to_string_lossy().into_owned()
        }

        /// The three variables with this layout's paths.
        fn variables(&self) -> Vec<(&'static str, String)> {
            vec![
                (ISABELLE_HOME, Layout::text(&self.home)),
                (ISABELLE_USER_HOME, Layout::text(&self.user)),
                (RULES_DIR, Layout::text(&self.rules)),
            ]
        }
    }

    #[cfg(any(debug_assertions, target_os = "macos"))]
    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The key source this build accepts in a configuration that must pass.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    fn accepted_key() -> &'static str {
        if cfg!(debug_assertions) {
            "test"
        } else {
            "keychain"
        }
    }

    /// The data directory and the key source, then `extra`.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    fn with_base<'a>(dir: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut list = vec![(DATA_DIR, dir), (KEY_SOURCE, accepted_key())];
        list.extend_from_slice(extra);
        list
    }

    /// PW c40: the three Isabelle variables are read together or not at all.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    #[test]
    fn isabelle_variables_are_all_or_none() {
        let dir = absolute();
        let layout = Layout::new("all-or-none");
        let all = layout.variables();
        let none = config_from(vars(&with_base(&dir, &[]))).unwrap();
        assert_eq!(none.isabelle, None);
        let pairs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let config = config_from(vars(&with_base(&dir, &pairs))).unwrap();
        assert_eq!(
            config.isabelle,
            Some(IsabelleSettings {
                isabelle_home: layout.home.clone(),
                user_home: layout.user.clone(),
                rules_dir: layout.rules.clone(),
            })
        );
        for kept in [
            vec![0],
            vec![1],
            vec![2],
            vec![0, 1],
            vec![0, 2],
            vec![1, 2],
        ] {
            let some: Vec<(&str, &str)> = kept.iter().map(|&i| pairs[i]).collect();
            let line = refused(&with_base(&dir, &some));
            assert_eq!(line, NOT_TOGETHER, "{kept:?}");
            for name in [ISABELLE_HOME, ISABELLE_USER_HOME, RULES_DIR] {
                assert!(line.contains(name), "{line}");
            }
        }
    }

    /// PW c41: each Isabelle path is checked before anything starts, and the refusal names its
    /// variable and path.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    #[test]
    fn each_isabelle_path_is_checked_before_anything_starts() {
        let dir = absolute();
        let layout = Layout::new("paths");
        let home = Layout::text(&layout.home);
        let user = Layout::text(&layout.user);
        let rules = Layout::text(&layout.rules);
        let missing = Layout::text(&layout.root.join("missing"));
        let file = Layout::text(&layout.rules.join("ROOTS"));
        let cases: Vec<(&str, [&str; 3], &str)> = vec![
            (
                "a relative home",
                ["isabelle", &user, &rules],
                ISABELLE_HOME,
            ),
            (
                "a home with no bin/isabelle",
                [&user, &user, &rules],
                ISABELLE_HOME,
            ),
            ("a missing home", [&missing, &user, &rules], ISABELLE_HOME),
            (
                "a relative user home",
                [&home, "user", &rules],
                ISABELLE_USER_HOME,
            ),
            (
                "a missing user home",
                [&home, &missing, &rules],
                ISABELLE_USER_HOME,
            ),
            (
                "a user home that is a file",
                [&home, &file, &rules],
                ISABELLE_USER_HOME,
            ),
            ("relative rules", [&home, &user, "rules"], RULES_DIR),
            ("missing rules", [&home, &user, &missing], RULES_DIR),
        ];
        for (case, [h, u, r], named) in cases {
            let line = refused(&with_base(
                &dir,
                &[(ISABELLE_HOME, h), (ISABELLE_USER_HOME, u), (RULES_DIR, r)],
            ));
            assert!(
                line.starts_with(&format!("fabric-broker: {named} is ")),
                "{case}: {line}"
            );
            assert!(!line.contains('\n'), "{case}: {line}");
        }
        for file in [
            "ROOTS",
            "Fabric_Broker/ROOT",
            "Fabric_Broker/Fabric_Broker.thy",
        ] {
            let layout = Layout::new("rules-file");
            std::fs::remove_file(layout.rules.join(file)).unwrap();
            let all = layout.variables();
            let pairs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let line = refused(&with_base(&dir, &pairs));
            assert!(line.contains(RULES_DIR) && line.contains(file), "{line}");
            assert!(line.contains(&Layout::text(&layout.rules)), "{line}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let layout = Layout::new("not-executable");
            std::fs::set_permissions(
                layout.home.join("bin").join("isabelle"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            let all = layout.variables();
            let pairs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let line = refused(&with_base(&dir, &pairs));
            assert!(
                line.contains(ISABELLE_HOME) && line.contains("may not be executed"),
                "{line}"
            );
        }
    }

    /// PW c42: the practice calendar runs only in a debug build with the synthetic agent and a
    /// configured Isabelle.
    #[cfg(debug_assertions)]
    #[test]
    fn practice_is_debug_synthetic_and_isabelle_only() {
        let dir = absolute();
        let layout = Layout::new("practice");
        let all = layout.variables();
        let pairs: Vec<(&str, &str)> = all.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut both = pairs.clone();
        both.push((SYNTHETIC_AGENT, "1"));
        assert!(
            config_from(vars(&with_base(&dir, &both)))
                .unwrap()
                .practice()
        );
        assert!(
            !config_from(vars(&with_base(&dir, &pairs)))
                .unwrap()
                .practice()
        );
        let synthetic_only = [(SYNTHETIC_AGENT, "1")];
        assert!(
            !config_from(vars(&with_base(&dir, &synthetic_only)))
                .unwrap()
                .practice()
        );
    }

    /// PW c42r: a release build never runs the practice calendar, whatever its configuration
    /// holds. Only a value is built here; nothing opens a store.
    #[cfg(all(not(debug_assertions), target_os = "macos"))]
    #[test]
    fn practice_is_never_on_in_release() {
        let layout = Layout::new("release");
        let config = Config {
            data_dir: PathBuf::from(absolute()),
            key_source: KeySource::Keychain,
            synthetic_agent: true,
            isabelle: Some(IsabelleSettings {
                isabelle_home: layout.home.clone(),
                user_home: layout.user.clone(),
                rules_dir: layout.rules.clone(),
            }),
        };
        assert!(!config.practice());
    }

    /// PW c43: the refusal of an unknown name lists the six names the broker reads (D27).
    #[test]
    fn the_refusal_lists_six_names() {
        let dir = absolute();
        let line = refused(&[(DATA_DIR, &dir), ("FABRIC_BROKER_ISABELLE", "x")]);
        for name in [
            DATA_DIR,
            KEY_SOURCE,
            SYNTHETIC_AGENT,
            ISABELLE_HOME,
            ISABELLE_USER_HOME,
            RULES_DIR,
        ] {
            assert!(line.contains(name), "{name}: {line}");
        }
    }

    #[test]
    fn the_variable_names_are_those_the_host_and_the_relay_spell() {
        // The relay's allowlist and the host spell these names too (fabric-desktop-relay); the
        // relay's test `allowlist_names_the_broker_variables` pins the same literals.
        assert_eq!(VARIABLE_PREFIX, "FABRIC_BROKER_");
        assert_eq!(DATA_DIR, "FABRIC_BROKER_DATA_DIR");
        assert_eq!(KEY_SOURCE, "FABRIC_BROKER_KEY_SOURCE");
        assert_eq!(SYNTHETIC_AGENT, "FABRIC_BROKER_SYNTHETIC_AGENT");
        assert_eq!(ISABELLE_HOME, "FABRIC_BROKER_ISABELLE_HOME");
        assert_eq!(ISABELLE_USER_HOME, "FABRIC_BROKER_ISABELLE_USER_HOME");
        assert_eq!(RULES_DIR, "FABRIC_BROKER_RULES_DIR");
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
