//! The parts of the gate that need no Isabelle: removing stale check theories, the theory the
//! crate was built with, whose fault an error is, and Isabelle's private temporary directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fabric_broker_policy::gate::{check_rounds, remove_stale_checks};
use fabric_broker_policy::report::sha256_hex;
use fabric_broker_policy::theory::{SESSION_ROOT, THEORY_TEXT};
use fabric_broker_policy::{Fault, PolicyError, built_theory_sha256};

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
}

/// An empty directory of its own for one test, under Cargo's scratch directory for tests.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("fabric-broker-policy-gate")
        .join(name);
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => panic!("cannot clear {}: {error}", dir.display()),
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn stale_check_theories_are_removed_and_nothing_else() {
    let dir = scratch("stale");
    for name in ["Check_000001.thy", "Check_999999.thy", "Check_.thy"] {
        fs::write(dir.join(name), "calendar data").unwrap();
    }
    // Not check theories, or not regular files directly in the directory: they stay.
    for name in [
        "Check_000001.thy.bak",
        "check_000002.thy",
        "Other.thy",
        "Check_1.txt",
    ] {
        fs::write(dir.join(name), "kept").unwrap();
    }
    fs::create_dir(dir.join("Check_dir.thy")).unwrap();
    fs::create_dir(dir.join("nested")).unwrap();
    fs::write(dir.join("nested").join("Check_000003.thy"), "kept").unwrap();

    assert_eq!(remove_stale_checks(&dir).unwrap(), 3);
    let mut left: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "Check_000001.thy.bak",
            "Check_1.txt",
            "Check_dir.thy",
            "Other.thy",
            "check_000002.thy",
            "nested"
        ]
    );
    assert!(dir.join("nested").join("Check_000003.thy").exists());
    // Again: nothing more to remove.
    assert_eq!(remove_stale_checks(&dir).unwrap(), 0);
}

#[test]
fn the_built_theory_is_the_repository_theory() {
    let session = repository().join("isabelle").join("Fabric_Broker");
    let theory = fs::read(session.join("Fabric_Broker.thy")).unwrap();
    assert_eq!(THEORY_TEXT.as_bytes(), theory.as_slice());
    assert_eq!(
        SESSION_ROOT,
        fs::read_to_string(session.join("ROOT")).unwrap()
    );
    assert_eq!(built_theory_sha256(), sha256_hex(&theory));
    assert_eq!(built_theory_sha256().len(), 64);
    assert!(
        built_theory_sha256()
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
}

/// Isabelle ends a check after this many looks, 50 ms apart; it reads 0 as "no limit".
#[test]
fn a_limit_becomes_a_number_of_looks_and_never_zero() {
    use std::time::Duration;

    assert_eq!(check_rounds(Duration::from_secs(30)), 600);
    assert_eq!(check_rounds(Duration::from_millis(50)), 1);
    assert_eq!(check_rounds(Duration::from_millis(51)), 2);
    assert_eq!(check_rounds(Duration::from_millis(1)), 1);
    assert_eq!(check_rounds(Duration::ZERO), 1);
}

#[test]
fn only_a_render_error_is_the_requests_fault() {
    let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let cases = [
        (PolicyError::Render("bad".to_owned()), Fault::Request),
        (PolicyError::Protocol("odd".to_owned()), Fault::Gate),
        (PolicyError::Isabelle("failed".to_owned()), Fault::Gate),
        (PolicyError::Io(io::Error::other("gone")), Fault::Gate),
        (PolicyError::Json(json), Fault::Gate),
    ];
    for (error, fault) in &cases {
        assert_eq!(Fault::of(error), *fault, "{error}");
    }
}

#[cfg(unix)]
mod private_tmp {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use fabric_broker_policy::gate::{TMP_PREFIX_SETTING, prepare_private_tmp};

    use super::scratch;

    fn isabelle_home(root: &Path, identifier: Option<&str>) -> std::path::PathBuf {
        let home = root.join("Isabelle");
        fs::create_dir_all(home.join("etc")).unwrap();
        if let Some(identifier) = identifier {
            // The distribution's file has no line feed.
            fs::write(home.join("etc").join("ISABELLE_IDENTIFIER"), identifier).unwrap();
        }
        home
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_settings_file_is_created_with_the_line() {
        let root = scratch("tmp-created");
        let home = isabelle_home(&root, Some("Isabelle2025-2"));
        let user_home = root.join("home");
        let tmp = prepare_private_tmp(&home, &user_home).unwrap();

        let home_user = user_home.join(".isabelle").join("Isabelle2025-2");
        assert_eq!(tmp, home_user.join("tmp"));
        let settings = fs::read_to_string(home_user.join("etc").join("settings")).unwrap();
        let lines: Vec<&str> = settings.lines().collect();
        assert_eq!(lines.len(), 2, "{settings}");
        assert!(lines[0].starts_with("# "));
        assert_eq!(lines[1], TMP_PREFIX_SETTING);
        assert_eq!(
            TMP_PREFIX_SETTING,
            "ISABELLE_TMP_PREFIX=\"$ISABELLE_HOME_USER/tmp\""
        );
        assert_eq!(mode(&tmp), 0o700);
    }

    #[test]
    fn without_an_identifier_the_user_directory_is_dot_isabelle() {
        let root = scratch("tmp-no-identifier");
        let home = isabelle_home(&root, None);
        let user_home = root.join("home");
        let tmp = prepare_private_tmp(&home, &user_home).unwrap();
        assert_eq!(tmp, user_home.join(".isabelle").join("tmp"));
        assert!(user_home.join(".isabelle/etc/settings").is_file());
    }

    #[test]
    fn other_settings_are_kept_and_the_line_is_appended_once() {
        let root = scratch("tmp-appended");
        let home = isabelle_home(&root, Some("Isabelle2025-2"));
        let user_home = root.join("home");
        let etc = user_home.join(".isabelle/Isabelle2025-2/etc");
        fs::create_dir_all(&etc).unwrap();
        // No final line feed, and a line that only mentions the setting.
        let own =
            "ML_OPTIONS=\"--minheap 1000\"\n# ISABELLE_TMP_PREFIX=\"$ISABELLE_HOME_USER/tmp\"";
        fs::write(etc.join("settings"), own).unwrap();

        prepare_private_tmp(&home, &user_home).unwrap();
        let first = fs::read_to_string(etc.join("settings")).unwrap();
        assert!(first.starts_with(&format!("{own}\n")), "{first}");
        assert_eq!(
            first
                .lines()
                .filter(|line| *line == TMP_PREFIX_SETTING)
                .count(),
            1
        );
        assert!(first.ends_with(&format!("{TMP_PREFIX_SETTING}\n")));

        // Again: the file and the directory's mode stay as they are.
        let tmp = user_home.join(".isabelle/Isabelle2025-2/tmp");
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).unwrap();
        prepare_private_tmp(&home, &user_home).unwrap();
        assert_eq!(fs::read_to_string(etc.join("settings")).unwrap(), first);
        assert_eq!(mode(&tmp), 0o700);
    }

    #[test]
    fn a_file_with_the_line_already_is_left_alone() {
        let root = scratch("tmp-present");
        let home = isabelle_home(&root, Some("Isabelle2025-2"));
        let user_home = root.join("home");
        let etc = user_home.join(".isabelle/Isabelle2025-2/etc");
        fs::create_dir_all(&etc).unwrap();
        let own = format!("{TMP_PREFIX_SETTING}\nISABELLE_BUILD_OPTIONS=\"\"\n");
        fs::write(etc.join("settings"), &own).unwrap();
        prepare_private_tmp(&home, &user_home).unwrap();
        assert_eq!(fs::read_to_string(etc.join("settings")).unwrap(), own);
    }

    #[test]
    fn a_tmp_that_is_not_a_directory_is_refused() {
        let root = scratch("tmp-symlink");
        let home = isabelle_home(&root, Some("Isabelle2025-2"));
        let user_home = root.join("home");
        let home_user = user_home.join(".isabelle/Isabelle2025-2");
        fs::create_dir_all(&home_user).unwrap();
        fs::create_dir(root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), home_user.join("tmp")).unwrap();
        assert!(prepare_private_tmp(&home, &user_home).is_err());
    }
}
