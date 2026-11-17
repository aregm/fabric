//! A fresh scratch directory per test, under the system's temporary directory, with a short
//! name so that the socket's full path stays a legal Unix socket name on macOS (design 9).

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The longest socket path a test may use: macOS keeps 104 bytes for the name, its NUL
/// included, so a path must stay under 104 bytes; the design asks for under 103.
pub const MAX_SOCKET_PATH: usize = 102;

/// A scratch directory: `root`, the socket's directory (mode 0700), the socket's path (not
/// created) and a key directory (mode 0700). Removed on drop.
pub struct Scratch {
    pub root: PathBuf,
    pub run_dir: PathBuf,
    pub socket: PathBuf,
    pub key_dir: PathBuf,
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the system's random source");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Makes a directory of mode 0700, whatever the umask.
pub fn private_dir(path: &Path) {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    set_mode(path, 0o700);
}

pub fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|e| panic!("chmod {:o} {}: {e}", mode, path.display()));
}

impl Scratch {
    /// `<tmp>/fmb-<8 hex>/run/agent.sock` and `<tmp>/fmb-<8 hex>/keys`.
    pub fn new() -> Scratch {
        Scratch::with_names("fmb-", "run", "keys")
    }

    /// The same with a space in every name: `<tmp>/fm b<8 hex>/r un/agent.sock` and
    /// `<tmp>/fm b<8 hex>/k ey` (PLAN 14 "a path containing spaces").
    pub fn spaced() -> Scratch {
        Scratch::with_names("fm b", "r un", "k ey")
    }

    fn with_names(prefix: &str, run: &str, keys: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("{prefix}{}", random_hex(4)));
        private_dir(&root);
        let run_dir = root.join(run);
        private_dir(&run_dir);
        let key_dir = root.join(keys);
        private_dir(&key_dir);
        let socket = run_dir.join("agent.sock");
        let length = socket.as_os_str().len();
        assert!(
            length <= MAX_SOCKET_PATH,
            "the socket path {} is {length} bytes, more than {MAX_SOCKET_PATH}",
            socket.display()
        );
        Scratch {
            root,
            run_dir,
            socket,
            key_dir,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A test may have loosened or tightened a mode; make everything removable first.
        let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
        let _ = fs::set_permissions(&self.run_dir, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.root);
    }
}
