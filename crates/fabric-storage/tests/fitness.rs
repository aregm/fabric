//! s16: a fitness test over the store's source (design 3.5, D3, 9.2): only `apply` writes the
//! projection tables. `WriteTx::apply` is the only projection write, so replay can rebuild
//! the projection from the ledger.
//!
//! The scan reads every `.rs` file under `crates/fabric-storage/src`, drops whole-line `//`
//! comments, splits the text into lowercase identifier tokens and looks for SQL that writes a
//! table: `INSERT [OR x] INTO t`, `REPLACE INTO t`, `UPDATE [OR x] t`, `DELETE FROM t`
//! (with an optional `main.` or `temp.` prefix). SQL built from a table name held in a
//! variable is not seen; the review covers that.

use std::path::{Path, PathBuf};

/// The projection tables of section 3.5: every table except `meta`, `ledger` and `audit`.
const PROJECTION_TABLES: [&str; 17] = [
    "runs",
    "counters",
    "profile_versions",
    "profile_state",
    "preferences",
    "recipient_sets",
    "recipient_set_versions",
    "accounts",
    "agents",
    "sessions",
    "requests",
    "grants",
    "proof_reports",
    "authorizations",
    "effects",
    "attempts",
    "commands",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Lowercase identifier tokens of `source`, without whole-line `//` comments.
fn tokens(source: &str) -> Vec<String> {
    let code: Vec<&str> = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();
    code.join("\n")
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// The projection tables `source` writes with literal SQL.
fn written_tables(source: &str) -> Vec<String> {
    let tokens = tokens(source);
    let at = |i: usize| tokens.get(i).map_or("", String::as_str);
    let table_at = |i: usize| -> Option<String> {
        let name = match at(i) {
            "main" | "temp" => at(i + 1),
            other => other,
        };
        PROJECTION_TABLES.contains(&name).then(|| name.to_owned())
    };
    let mut written = Vec::new();
    for i in 0..tokens.len() {
        let target = match at(i) {
            "insert" => {
                let into = if at(i + 1) == "or" { i + 3 } else { i + 1 };
                (at(into) == "into").then(|| table_at(into + 1)).flatten()
            }
            // `INSERT OR REPLACE INTO` is counted at its `insert`.
            "replace" if at(i + 1) == "into" && (i == 0 || at(i - 1) != "or") => table_at(i + 2),
            "update" => {
                let table = if at(i + 1) == "or" { i + 3 } else { i + 1 };
                table_at(table)
            }
            "delete" if at(i + 1) == "from" => table_at(i + 2),
            _ => None,
        };
        written.extend(target);
    }
    written
}

/// Whether `path` is the `apply` module (`src/apply.rs` or a file under `src/apply/`).
fn is_apply(src: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(src).unwrap();
    relative == Path::new("apply.rs") || relative.starts_with("apply")
}

/// s16 (3.5, D3): no source file of the store other than `apply` writes a projection table;
/// `apply` does, which shows the scan finds what it looks for.
#[test]
fn only_apply_writes_projection_tables() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no source files under {}", src.display());

    let mut offenders = Vec::new();
    let mut written_by_apply = Vec::new();
    for path in &files {
        let written = written_tables(&std::fs::read_to_string(path).unwrap());
        if is_apply(&src, path) {
            written_by_apply.extend(written);
        } else if !written.is_empty() {
            offenders.push(format!("{}: {written:?}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "projection tables written outside apply:\n{}",
        offenders.join("\n")
    );
    assert!(
        !written_by_apply.is_empty(),
        "apply writes no projection table that the scan can see"
    );
}

/// The scan itself: it sees the statement forms it promises and ignores comments and names
/// that only contain a table name.
#[test]
fn the_scan_finds_projection_writes_and_ignores_comments() {
    let source = r#"
        // UPDATE agents SET paused = 1
        const A: &str = "INSERT INTO agents (agent_id) VALUES (?1)";
        const B: &str = "insert or replace into main.grants(grant_id) values (?1)";
        const C: &str = "UPDATE requests SET state = ?1";
        const D: &str = "DELETE FROM sessions WHERE session_id = ?1";
        const E: &str = "REPLACE INTO commands VALUES (?1)";
        const F: &str = "INSERT INTO ledger (seq) VALUES (?1)";
        fn update_agents() {}
    "#;
    assert_eq!(
        written_tables(source),
        vec!["agents", "grants", "requests", "sessions", "commands"]
    );
}
