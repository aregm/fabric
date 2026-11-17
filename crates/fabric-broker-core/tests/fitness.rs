//! The forbidden-dependency checks of design section 2 (a manifest scan; PLAN 4.3, 4.4; RM RD-9,
//! RD-11). The demo's `AuditLog` and `fabric-token`'s `ReplayCache` stay out of the broker
//! because the crates that hold them are not dependencies (DEF-1, DEF-4).

use std::path::{Path, PathBuf};

/// Network, async and model crates: the authority core and its store do no I/O but the store's
/// file, and never call a model (design section 2).
const NETWORK_ASYNC_MODEL: &[&str] = &[
    "reqwest",
    "hyper",
    "http",
    "h2",
    "ureq",
    "isahc",
    "surf",
    "curl",
    "attohttpc",
    "axum",
    "actix-web",
    "warp",
    "tonic",
    "tokio",
    "async-std",
    "smol",
    "futures",
    "async-trait",
    "mio",
    "openai",
    "async-openai",
    "anthropic",
    "anthropic-sdk",
    "llm",
    "ollama-rs",
    "candle-core",
    "tch",
    "ort",
];

/// The workspace crates the store and the core must not use.
const POLICY_AND_DEMO: &[&str] = &[
    "fabric-policy",
    "fabric-token",
    "fabric-broker-policy",
    "fabric-demo",
    "fabric-core",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists")
}

/// The package names a manifest depends on, in every dependency section (normal, dev, build,
/// and per target), with renames resolved to the real package.
fn dependencies(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let header = line.trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = header.rsplitn(2, '.').collect();
            in_dependencies = header.ends_with("dependencies");
            // `[dependencies.name]` or `[target.'..'.dependencies.name]`.
            if !in_dependencies && parts.len() == 2 && parts[1].ends_with("dependencies") {
                names.push(parts[0].trim_matches('"').to_owned());
            }
            continue;
        }
        // `name = { package = "real", .. }` or `package = "real"` in a dependency table.
        if let Some(at) = line.find("package")
            && let Some(value) = line[at + "package".len()..].trim_start().strip_prefix('=')
            && let Some(real) = value.split('"').nth(1)
        {
            names.push(real.to_owned());
        }
        if in_dependencies && let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().trim_matches('"').to_owned());
        }
    }
    names
}

fn is_forbidden(dependency: &str, forbidden: &str) -> bool {
    dependency == forbidden || dependency.starts_with(&format!("{forbidden}-"))
}

fn check(manifest: &str, forbidden: &[&[&str]], extra: impl Fn(&str) -> bool) {
    let path = root().join(manifest);
    let names = dependencies(&path);
    assert!(
        !names.is_empty(),
        "{manifest}: the scan found no dependency, so it read nothing"
    );
    for name in &names {
        for list in forbidden {
            for f in *list {
                assert!(
                    !is_forbidden(name, f),
                    "{manifest} depends on {name}, which design section 2 forbids"
                );
            }
        }
        assert!(
            !extra(name),
            "{manifest} depends on {name}, which design section 2 forbids"
        );
    }
}

/// Design section 2: the store, the core, the broker, the relay and the host each keep out of
/// the crates the table forbids them.
#[test]
fn forbidden_dependencies_are_absent() {
    check(
        "crates/fabric-storage/Cargo.toml",
        &[
            POLICY_AND_DEMO,
            NETWORK_ASYNC_MODEL,
            &["fabric-broker-core"],
        ],
        |_| false,
    );
    check(
        "crates/fabric-broker-core/Cargo.toml",
        &[POLICY_AND_DEMO, NETWORK_ASYNC_MODEL],
        |name| name == "tauri" || name.starts_with("tauri-"),
    );
    check(
        "apps/fabric-broker/Cargo.toml",
        &[&["fabric-demo", "fabric-token", "fabric-policy"]],
        |_| false,
    );
    check(
        "apps/fabric-desktop/relay/Cargo.toml",
        &[&["fabric-storage", "fabric-broker-core"]],
        |_| false,
    );
    check(
        "apps/fabric-desktop/src-tauri/Cargo.toml",
        &[&["fabric-storage", "fabric-broker-core"]],
        |_| false,
    );
}

/// The scan itself: it finds names in every form a manifest writes them.
#[test]
fn the_manifest_scan_reads_every_dependency_form() {
    let dir = std::env::temp_dir().join(format!("fabric-fitness-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n\n[dependencies]\nplain = \"1\"\nrenamed = { package = \"real\", version = \"1\" }\n\
         \n[dev-dependencies]\ndev = \"1\"\n\n[target.'cfg(unix)'.dependencies]\ntargeted = \"1\"\n\
         \n[dependencies.table]\nversion = \"1\"\n\n[lints]\nworkspace = true\n",
    )
    .expect("write a manifest");
    let names = dependencies(&manifest);
    let _ = std::fs::remove_dir_all(&dir);
    for expected in ["plain", "renamed", "real", "dev", "targeted", "table"] {
        assert!(
            names.iter().any(|n| n == expected),
            "missed {expected}: {names:?}"
        );
    }
    assert!(!names.iter().any(|n| n == "name" || n == "workspace"));
}

/// The production text of a source file: comment lines and a trailing test module left out.
fn production(path: &Path) -> String {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| {
            w[0].starts_with("#[cfg(")
                && w[0].contains("test")
                && (w[1].starts_with("mod ") || w[1].starts_with("pub(crate) mod "))
        })
        .unwrap_or(lines.len());
    lines[..end]
        .iter()
        .filter(|line| !line.trim_start().starts_with("//"))
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of the first `fn name(` in `text`, braces included.
fn body<'a>(text: &'a str, name: &str) -> &'a str {
    let at = text
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name}"));
    let open = at + text[at..].find('{').expect("a body");
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open..=open + i];
                }
            }
            _ => {}
        }
    }
    panic!("the body of fn {name} does not end")
}

/// Every production source file of the core, by its path under `src`.
fn sources(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read the source directory")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|n| n != "tests.rs")
        {
            let name = path
                .strip_prefix(base)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((name, production(&path)));
        }
    }
}

/// c34. 5.7: only `undecide` moves a request to `undecided`, so every such move states its
/// cause. No production file but the reducer and `undecide` names the event, and only `undecide`
/// passes a cause to `move_request_with`; `move_request` passes none.
#[test]
fn only_undecide_moves_a_request_to_undecided() {
    let src = root().join("crates/fabric-broker-core/src");
    let mut files = Vec::new();
    sources(&src, &src, &mut files);
    assert!(files.len() > 10, "the scan read {} files", files.len());
    let event = "Event::Undecide";
    let call = "move_request_with(";
    for (name, text) in &files {
        let mut allowed_events = 0;
        let mut allowed_calls = 0;
        match name.as_str() {
            "service/reducer.rs" => allowed_events = text.matches(event).count(),
            "service/pipeline.rs" => {
                let undecide = body(text, "undecide");
                assert_eq!(
                    undecide.matches(event).count(),
                    1,
                    "undecide names the event"
                );
                assert!(
                    undecide.contains("Some(cause)"),
                    "undecide passes its cause"
                );
                allowed_events = 1;
                allowed_calls = undecide.matches(call).count();
                let advance = body(text, "advance");
                assert_eq!(advance.matches(call).count(), 1);
                assert!(
                    advance.contains("due_at_ms, None)"),
                    "advance passes a cause to move_request_with"
                );
                allowed_calls += 1;
            }
            "service/requests.rs" => {
                let wrapper = body(text, "move_request");
                assert_eq!(wrapper.matches(call).count(), 1);
                assert!(
                    wrapper.contains("due_at_ms, None)"),
                    "move_request passes a cause to move_request_with"
                );
                // The wrapper's call and the definition.
                allowed_calls = 2;
            }
            _ => {}
        }
        assert_eq!(
            text.matches(event).count(),
            allowed_events,
            "{name} names {event} outside the reducer and undecide"
        );
        assert_eq!(
            text.matches(call).count(),
            allowed_calls,
            "{name} calls move_request_with outside undecide, advance and move_request"
        );
    }
}
