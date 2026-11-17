//! The agent's key, its file store in debug builds, and the pairing code (W06 5.3, 5.4).
//!
//! The key is Ed25519. Its 32-byte seed comes from the system's random source and is held in
//! `Zeroizing` memory; `ed25519_dalek::SigningKey` wipes its own copy when dropped. The public
//! key and signatures are the only things derived from it that leave the bridge.

use std::fmt;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest as _, Sha256};

/// The domain of the pairing code (W06 5.3).
pub const PAIRING_DOMAIN: &str = "fabric-desktop/1\nagent_key\n";

/// An agent's signing key.
pub struct AgentKey {
    signing: SigningKey,
}

impl fmt::Debug for AgentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentKey(..)")
    }
}

impl AgentKey {
    /// The key of a seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> AgentKey {
        AgentKey {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// `ed25519:` and the 32-byte public key in 64 lowercase hexadecimal digits.
    #[must_use]
    pub fn public_key_text(&self) -> String {
        format!(
            "ed25519:{}",
            crate::hex(&self.signing.verifying_key().to_bytes())
        )
    }

    /// The pairing code of this key.
    #[must_use]
    pub fn pairing_code(&self) -> String {
        pairing_code(&self.public_key_text())
    }

    /// The Ed25519 signature of `message` in 128 lowercase hexadecimal digits.
    #[must_use]
    pub fn sign_hex(&self, message: &[u8]) -> String {
        crate::hex(&self.signing.sign(message).to_bytes())
    }

    /// The 32-byte public key.
    #[must_use]
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }
}

/// The pairing code of a public key text: the first 12 hexadecimal digits, in capitals, of
/// `SHA-256(PAIRING_DOMAIN + public_key_text)`, in three groups of four (W06 5.3).
#[must_use]
pub fn pairing_code(public_key_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PAIRING_DOMAIN.as_bytes());
    hasher.update(public_key_text.as_bytes());
    let digits = crate::hex(&hasher.finalize()[..6]).to_ascii_uppercase();
    format!("{}-{}-{}", &digits[0..4], &digits[4..8], &digits[8..12])
}

/// Whether `text` has the pairing code's form `XXXX-XXXX-XXXX` in uppercase hexadecimal.
#[must_use]
pub fn is_pairing_code(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 14
        && b.iter().enumerate().all(|(i, &c)| {
            if i == 4 || i == 9 {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'A'..=b'F').contains(&c)
            }
        })
}

/// Why a key could not be had. Every message is fixed: none repeats a file's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// This build has no key store (W06 5.4: the Keychain store comes in phase 2).
    NoStore,
    /// The system's random source failed.
    Random,
    /// The key directory is missing, a link, or not a directory.
    DirectoryMissing,
    /// The key directory's mode is not exactly 0700.
    DirectoryMode,
    /// The key directory belongs to another user.
    DirectoryOwner,
    /// The key file is a link or not a plain file.
    FileNotPlain,
    /// The key file belongs to another user.
    FileOwner,
    /// Someone other than the owner has a permission on the key file.
    FileMode,
    /// The key file is not 64 lowercase hexadecimal digits and a line feed.
    FileMalformed,
    /// The key file changed between its check and its reading.
    FileChanged,
    /// Reading or writing failed.
    Io,
}

impl KeyError {
    /// A fixed name of the failure, for stderr.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            KeyError::NoStore => "no key store in this build",
            KeyError::Random => "the random source failed",
            KeyError::DirectoryMissing => "the key directory is not a directory",
            KeyError::DirectoryMode => "the key directory's mode is not 0700",
            KeyError::DirectoryOwner => "the key directory belongs to another user",
            KeyError::FileNotPlain => "the key file is not a plain file",
            KeyError::FileOwner => "the key file belongs to another user",
            KeyError::FileMode => "the key file has permissions for others",
            KeyError::FileMalformed => "the key file is malformed",
            KeyError::FileChanged => "the key file changed while it was read",
            KeyError::Io => "the key file could not be read or written",
        }
    }
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for KeyError {}

/// The key file store of debug builds with `--key-dir` (W06 5.4).
#[cfg(unix)]
pub mod file {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use fabric_desktop_contracts::scalar::Id;
    use zeroize::Zeroizing;

    use super::{AgentKey, KeyError};

    /// The lowercase hexadecimal digits, indexed by nibble.
    const HEX: &[u8; 16] = b"0123456789abcdef";

    /// A key file's bytes: 64 lowercase hexadecimal digits and a line feed.
    const FILE_BYTES: usize = 65;

    fn new_seed() -> Result<Zeroizing<[u8; 32]>, KeyError> {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(seed.as_mut()).map_err(|_| KeyError::Random)?;
        Ok(seed)
    }

    /// The file's bytes for `seed`, built in place with no temporary copy.
    fn encode_seed(seed: &[u8; 32]) -> Zeroizing<[u8; FILE_BYTES]> {
        let mut text = Zeroizing::new([0u8; FILE_BYTES]);
        for (i, byte) in seed.iter().enumerate() {
            text[2 * i] = HEX[usize::from(byte >> 4)];
            text[2 * i + 1] = HEX[usize::from(byte & 0x0f)];
        }
        text[FILE_BYTES - 1] = b'\n';
        text
    }

    fn euid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    /// A directory of key files, `<dir>/<id>.key`.
    #[derive(Clone, Debug)]
    pub struct FileKeyStore {
        dir: PathBuf,
    }

    impl FileKeyStore {
        #[must_use]
        pub fn new(dir: PathBuf) -> FileKeyStore {
            FileKeyStore { dir }
        }

        /// The key of `id`: the stored one, or a new one when there is none. Only "there is
        /// no key" leads to a new key; any other failure is an error (W06 5.4).
        pub fn load_or_create(&self, id: &Id) -> Result<AgentKey, KeyError> {
            self.check_directory()?;
            match self.load(id)? {
                Some(key) => Ok(key),
                None => self.store_new(id, &*new_seed()?),
            }
        }

        fn path(&self, id: &Id) -> PathBuf {
            self.dir.join(format!("{}.key", id.as_str()))
        }

        /// The directory is a directory (not a link) of mode exactly 0700 owned by this user.
        fn check_directory(&self) -> Result<(), KeyError> {
            let meta = fs::symlink_metadata(&self.dir).map_err(|_| KeyError::DirectoryMissing)?;
            if !meta.file_type().is_dir() {
                return Err(KeyError::DirectoryMissing);
            }
            if meta.uid() != euid() {
                return Err(KeyError::DirectoryOwner);
            }
            if meta.mode() & 0o7777 != 0o700 {
                return Err(KeyError::DirectoryMode);
            }
            Ok(())
        }

        /// Reads the stored key, or `None` when there is no file.
        fn load(&self, id: &Id) -> Result<Option<AgentKey>, KeyError> {
            let path = self.path(id);
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(_) => return Err(KeyError::Io),
            };
            check_file(&meta)?;
            let mut file = File::open(&path).map_err(|_| KeyError::Io)?;
            let opened = file.metadata().map_err(|_| KeyError::Io)?;
            if opened.dev() != meta.dev() || opened.ino() != meta.ino() {
                return Err(KeyError::FileChanged);
            }
            check_file(&opened)?;
            // One byte more than a key file holds, so that a longer file is seen as malformed.
            let mut text = Zeroizing::new([0u8; FILE_BYTES + 1]);
            let mut filled = 0;
            while filled < text.len() {
                match file.read(&mut text[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return Err(KeyError::Io),
                }
            }
            let seed = parse_seed(&text[..filled])?;
            Ok(Some(AgentKey::from_seed(&seed)))
        }

        /// Stores `seed` as the key of `id` without replacing: a temporary file of mode 0600 in
        /// the directory, linked to its name. When the name exists, another maker won the race,
        /// and the stored key is read instead (W06 5.4).
        pub fn store_new(&self, id: &Id, seed: &[u8; 32]) -> Result<AgentKey, KeyError> {
            self.check_directory()?;
            let suffix = crate::random_128().ok_or(KeyError::Random)?;
            let temporary = self
                .dir
                .join(format!(".{}.key.{}", id.as_str(), crate::hex(&suffix)));
            let written = write_temporary(&temporary, seed);
            let linked = written.and_then(|()| match fs::hard_link(&temporary, self.path(id)) {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
                Err(_) => Err(KeyError::Io),
            });
            let _ = fs::remove_file(&temporary);
            if linked? {
                Ok(AgentKey::from_seed(seed))
            } else {
                self.load(id)?.ok_or(KeyError::Io)
            }
        }
    }

    fn write_temporary(path: &Path, seed: &[u8; 32]) -> Result<(), KeyError> {
        let text = encode_seed(seed);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| KeyError::Io)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| KeyError::Io)?;
        file.write_all(text.as_slice()).map_err(|_| KeyError::Io)?;
        file.sync_all().map_err(|_| KeyError::Io)
    }

    /// A key file is a plain file, owned by this user, with no permission for anyone else.
    fn check_file(meta: &fs::Metadata) -> Result<(), KeyError> {
        if !meta.file_type().is_file() {
            return Err(KeyError::FileNotPlain);
        }
        if meta.uid() != euid() {
            return Err(KeyError::FileOwner);
        }
        if meta.mode() & 0o077 != 0 {
            return Err(KeyError::FileMode);
        }
        Ok(())
    }

    /// 64 lowercase hexadecimal digits and a line feed.
    fn parse_seed(text: &[u8]) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        if text.len() != FILE_BYTES || text[FILE_BYTES - 1] != b'\n' {
            return Err(KeyError::FileMalformed);
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut seed = Zeroizing::new([0u8; 32]);
        for (i, pair) in text[..64].chunks(2).enumerate() {
            let (Some(hi), Some(lo)) = (digit(pair[0]), digit(pair[1])) else {
                return Err(KeyError::FileMalformed);
            };
            seed[i] = (hi << 4) | lo;
        }
        Ok(seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // k02: the pairing code of a fixed key, twelve digits in three groups.
    #[test]
    fn k02_pairing_code() {
        let key = AgentKey::from_seed(&[7u8; 32]);
        let text = key.public_key_text();
        assert!(text.starts_with("ed25519:"));
        assert_eq!(text.len(), 8 + 64);
        let code = key.pairing_code();
        assert!(is_pairing_code(&code), "{code}");
        // Recomputed by hand from the rule of W06 5.3.
        let mut h = Sha256::new();
        h.update(b"fabric-desktop/1\nagent_key\n");
        h.update(text.as_bytes());
        let full = crate::hex(&h.finalize()).to_ascii_uppercase();
        assert_eq!(
            code,
            format!("{}-{}-{}", &full[0..4], &full[4..8], &full[8..12])
        );
        assert_ne!(code, AgentKey::from_seed(&[8u8; 32]).pairing_code());
        // RFC 8032, section 7.1, test 1: the public key of that seed.
        let rfc = AgentKey::from_seed(&[
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ]);
        assert_eq!(
            rfc.public_key_text(),
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert!(!is_pairing_code("ab12-CD34-EF56"));
        assert!(!is_pairing_code("AB12CD34EF56"));
    }

    // k03: a key file is made 0600 in a 0700 directory and read back; a link, a file others
    // can read and a loose directory are refused; two makers at once end with one key; an
    // error never repeats the file's content.
    #[cfg(unix)]
    #[test]
    fn k03_key_file() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::sync::{Arc, Barrier};

        use fabric_desktop_contracts::scalar::Id;

        use super::file::FileKeyStore;

        let root = std::env::temp_dir().join(format!(
            "fmb-k03-{}",
            crate::hex(&crate::random_128().unwrap())
        ));
        let dir = root.join("keys");
        std::fs::create_dir_all(&dir).unwrap();
        let set_mode = |p: &std::path::Path, m: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        };
        set_mode(&dir, 0o700);
        let store = FileKeyStore::new(dir.clone());
        let id = Id::new("agent-test").unwrap();

        let made = store.load_or_create(&id).unwrap();
        let path = dir.join("agent-test.key");
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.file_type().is_file());
        assert_eq!(meta.mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap().len(), 65);
        let again = store.load_or_create(&id).unwrap();
        assert_eq!(made.public_key_text(), again.public_key_text());
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

        // A second maker that loses the race reads the stored key.
        let other = store.store_new(&id, &[9u8; 32]).unwrap();
        assert_eq!(other.public_key_text(), made.public_key_text());

        // Two makers at once end with one key.
        let race = Id::new("agent-race").unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let race = race.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.load_or_create(&race).unwrap().public_key_text()
                })
            })
            .collect();
        let keys: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(keys[0], keys[1]);

        // A file others can read is refused.
        set_mode(&path, 0o644);
        assert_eq!(store.load_or_create(&id).unwrap_err(), KeyError::FileMode);
        set_mode(&path, 0o600);

        // A link in place of the file is refused.
        let link_id = Id::new("agent-link").unwrap();
        std::os::unix::fs::symlink(&path, dir.join("agent-link.key")).unwrap();
        assert_eq!(
            store.load_or_create(&link_id).unwrap_err(),
            KeyError::FileNotPlain
        );

        // A malformed file is refused, and the error does not repeat its content.
        let bad_id = Id::new("agent-bad").unwrap();
        let bad = dir.join("agent-bad.key");
        std::fs::write(&bad, "SECRET-CONTENT-0123\n").unwrap();
        set_mode(&bad, 0o600);
        let e = store.load_or_create(&bad_id).unwrap_err();
        assert_eq!(e, KeyError::FileMalformed);
        assert!(!format!("{e} {e:?}").contains("SECRET"));
        // And it is not replaced.
        assert_eq!(std::fs::read(&bad).unwrap(), b"SECRET-CONTENT-0123\n");

        // A loose directory is refused.
        set_mode(&dir, 0o755);
        assert_eq!(
            store.load_or_create(&id).unwrap_err(),
            KeyError::DirectoryMode
        );
        set_mode(&dir, 0o700);

        // A missing directory is refused, not created.
        let missing = FileKeyStore::new(root.join("missing"));
        assert_eq!(
            missing.load_or_create(&id).unwrap_err(),
            KeyError::DirectoryMissing
        );
        assert!(!root.join("missing").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
