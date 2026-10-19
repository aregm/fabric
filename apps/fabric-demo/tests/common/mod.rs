//! Helpers shared by the tests that run the real `fabric-demo` commands.

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use fabric_policy::time::parse_utc;
use fabric_policy::{Action, ActionVerdict, CheckKind, CheckReport, Verdict};

pub fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

/// A fresh scratch directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

pub fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// A report as an authorize check that permitted `action` would produce it.
pub fn permit(action: &Action) -> CheckReport {
    CheckReport {
        kind: CheckKind::Authorize,
        theory: "Check_000001".to_owned(),
        proof_id: "ab".repeat(32),
        policy_sha256: "cd".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: vec![ActionVerdict {
            action: action.clone(),
            verdict: Verdict::Permit,
            rules: Vec::new(),
        }],
        theory_text: String::new(),
    }
}

/// A running command, killed when dropped.
pub struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `fabric-demo <command> --config <config>` and returns it with the URL from its first
/// line, which must start with `<prefix> listening on `.
pub fn start(command: &str, config: &Path, prefix: &str) -> (Running, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fabric-demo"))
        .args([command, "--config"])
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the command starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!("`{command}` exited: {stderr}");
    }
    let url = line
        .trim()
        .strip_prefix(&format!("{prefix} listening on "))
        .unwrap_or_else(|| panic!("unexpected first line {line:?}"))
        .to_owned();
    (Running(child), url)
}
