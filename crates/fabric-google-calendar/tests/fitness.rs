//! The forbidden-dependency checks of design section 2.1 (g46): a manifest scan, as
//! `crates/fabric-broker-core/tests/fitness.rs` does for W04.
//!
//! - This crate must not depend on `fabric-storage`, `fabric-policy`, `fabric-broker-policy`,
//!   `fabric-token`, `fabric-demo`, `fabric-core` or Tauri, in any section.
//! - `fabric-google-fake` may appear only under `[dev-dependencies]`, in this crate and in every
//!   other workspace member.
//! - `fabric-google-fake` itself depends on no `fabric-*` crate except the contract crate.

use std::path::{Path, PathBuf};

const FAKE: &str = "fabric-google-fake";
const CONTRACT: &str = "fabric-desktop-contracts";

/// The workspace crates this crate must not use (2.1).
const FORBIDDEN_FOR_ADAPTER: &[&str] = &[
    "fabric-storage",
    "fabric-policy",
    "fabric-broker-policy",
    "fabric-token",
    "fabric-demo",
    "fabric-core",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists")
}

/// One dependency of a manifest: its package name and whether its section is a
/// `dev-dependencies` section.
#[derive(Debug, PartialEq, Eq)]
struct Dependency {
    name: String,
    dev: bool,
}

/// The dependencies of a manifest text, in every section (normal, dev, build, workspace, and
/// per target), with renames resolved to the real package. A renamed dependency yields both the
/// key and the real name.
fn dependencies_of(text: &str) -> Vec<Dependency> {
    /// Where the scan is.
    enum At {
        /// Not in a dependency table.
        Elsewhere,
        /// In a `[..dependencies]` table: every key is a dependency.
        List { dev: bool },
        /// In a `[..dependencies.name]` table: only `package = ".."` names one more.
        One { dev: bool },
    }
    let mut found = Vec::new();
    let mut at = At::Elsewhere;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let header = line.trim_matches(|c| c == '[' || c == ']').trim();
            at = At::Elsewhere;
            if header.ends_with("dependencies") {
                at = At::List {
                    dev: header.ends_with("dev-dependencies"),
                };
            } else if let Some((table, name)) = header.rsplit_once('.')
                && table.ends_with("dependencies")
            {
                let dev = table.ends_with("dev-dependencies");
                found.push(Dependency {
                    name: name.trim_matches('"').to_owned(),
                    dev,
                });
                at = At::One { dev };
            }
            continue;
        }
        let dev = match at {
            At::Elsewhere => continue,
            At::List { dev } | At::One { dev } => dev,
        };
        // `package = "real"`, inline or alone in a `[..dependencies.name]` table.
        if let Some(p) = line.find("package")
            && let Some(value) = line[p + "package".len()..].trim_start().strip_prefix('=')
            && let Some(real) = value.split('"').nth(1)
        {
            found.push(Dependency {
                name: real.to_owned(),
                dev,
            });
        }
        if matches!(at, At::List { .. })
            && let Some((name, _)) = line.split_once('=')
        {
            found.push(Dependency {
                name: name.trim().trim_matches('"').to_owned(),
                dev,
            });
        }
    }
    found
}

fn dependencies(manifest: &Path) -> Vec<Dependency> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
    dependencies_of(&text)
}

/// The workspace members named in the root manifest, and the excluded Tauri host, which is a
/// workspace of its own in the same repository.
fn member_manifests() -> Vec<PathBuf> {
    let root = root();
    let text = std::fs::read_to_string(root.join("Cargo.toml")).expect("the root manifest");
    let start = text.find("members").expect("a members list");
    let list = &text[start..text[start..].find(']').map(|e| start + e).expect("its end")];
    let mut manifests: Vec<PathBuf> = list
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|member| root.join(member).join("Cargo.toml"))
        .collect();
    let tauri = root.join("apps/fabric-desktop/src-tauri/Cargo.toml");
    if tauri.exists() {
        manifests.push(tauri);
    }
    manifests.push(root.join("Cargo.toml"));
    manifests
}

fn is_tauri(name: &str) -> bool {
    name == "tauri" || name.starts_with("tauri-")
}

#[test]
fn the_adapter_keeps_out_of_the_forbidden_crates() {
    let manifest = root().join("crates/fabric-google-calendar/Cargo.toml");
    let deps = dependencies(&manifest);
    assert!(
        deps.iter().any(|d| d.name == CONTRACT),
        "the scan found the contract crate, so it read the manifest"
    );
    for d in &deps {
        assert!(
            !FORBIDDEN_FOR_ADAPTER.contains(&d.name.as_str()) && !is_tauri(&d.name),
            "fabric-google-calendar depends on {}, which design 2.1 forbids",
            d.name
        );
    }
}

#[test]
fn the_fake_is_only_ever_a_dev_dependency_of_a_workspace_member() {
    let manifests = member_manifests();
    assert!(
        manifests.len() > 10,
        "the members list was read: {manifests:?}"
    );
    for manifest in manifests {
        for d in dependencies(&manifest) {
            assert!(
                d.name != FAKE || d.dev,
                "{} depends on {FAKE} outside [dev-dependencies] (design 2.1)",
                manifest.display()
            );
        }
    }
}

#[test]
fn the_fake_depends_on_no_fabric_crate_but_the_contract() {
    let manifest = root().join("crates/fabric-google-fake/Cargo.toml");
    let deps = dependencies(&manifest);
    assert!(!deps.is_empty(), "the scan read the fake's manifest");
    for d in deps {
        assert!(
            !d.name.starts_with("fabric-") || d.name == CONTRACT,
            "{FAKE} depends on {}, which design 2.1 forbids",
            d.name
        );
    }
}

/// The scan itself: it finds names and their sections in every form a manifest writes them.
#[test]
fn the_manifest_scan_reads_every_dependency_form_and_its_section() {
    let text = "[package]\nname = \"x\"\n\n[dependencies]\nplain = \"1\"\n\
                renamed = { package = \"real\", version = \"1\" }\n\
                \n[dev-dependencies]\ndev = \"1\"\ndev-renamed = { package = \"dev-real\" }\n\
                \n[target.'cfg(unix)'.dependencies]\ntargeted = \"1\"\n\
                \n[target.'cfg(unix)'.dev-dependencies]\ntargeted-dev = \"1\"\n\
                \n[build-dependencies]\nbuild = \"1\"\n\
                \n[dependencies.table]\nversion = \"1\"\n\
                \n[dev-dependencies.dev-table]\npackage = \"dev-table-real\"\nversion = \"1\"\n\
                \n[workspace.dependencies]\nshared = \"1\"\n\
                \n[lints]\nworkspace = true\n";
    let deps = dependencies_of(text);
    let has = |name: &str, dev: bool| deps.iter().any(|d| d.name == name && d.dev == dev);
    for normal in [
        "plain", "renamed", "real", "targeted", "build", "table", "shared",
    ] {
        assert!(has(normal, false), "missed {normal}: {deps:?}");
    }
    for dev in [
        "dev",
        "dev-renamed",
        "dev-real",
        "targeted-dev",
        "dev-table",
        "dev-table-real",
    ] {
        assert!(has(dev, true), "missed {dev}: {deps:?}");
    }
    assert!(
        !deps
            .iter()
            .any(|d| d.name == "name" || d.name == "workspace" || d.name == "version"),
        "{deps:?}"
    );
}
