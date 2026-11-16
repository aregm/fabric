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
