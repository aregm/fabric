//! The Tauri build step. It declares the host's commands, so that the capability file can allow
//! exactly those and nothing else: a command that is not listed here cannot be allowed at all.

fn main() {
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "owner_attach",
            "owner_send",
            "broker_restart",
        ]));
    tauri_build::try_build(attributes).expect("the Tauri build step failed");
}
