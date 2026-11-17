//! The OAuth client and Google's client file (3.1).
//!
//! The file Google's console gives for a Desktop app client is a JSON object whose member
//! `installed` holds `client_id` and `client_secret`. Both are required; every other member,
//! `redirect_uris` among them, is ignored and never used. A failure names the path and a fixed
//! label, and nothing of the file's content (3.1, PD27).

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::secret::Secret;
use crate::secret::wipe::WipedJson;

/// The largest client file read. Google's file is well under 1 KiB.
pub const MAX_CLIENT_FILE: u64 = 64 * 1024;

/// The client id and secret of the Desktop app client (3.1). The secret is not a security
/// boundary (PLAN 10.1), and is still never logged, never in an error, never in the store.
pub struct ClientConfig {
    client_id: String,
    client_secret: Secret,
}

impl ClientConfig {
    /// A client from its two values, for tests and for the fake's clients. Empty values are
    /// refused as a missing member would be.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: Secret,
    ) -> Result<ClientConfig, ClientFileProblem> {
        let client_id = client_id.into();
        if client_id.is_empty() {
            return Err(ClientFileProblem::MissingClientId);
        }
        if client_secret.is_empty() {
            return Err(ClientFileProblem::MissingClientSecret);
        }
        Ok(ClientConfig {
            client_id,
            client_secret,
        })
    }

    /// Reads Google's client file at `path` (3.1). Where the broker gets the path is CO10.
    pub fn from_file(path: &Path) -> Result<ClientConfig, ClientFileError> {
        let error = |problem| ClientFileError {
            path: path.to_path_buf(),
            problem,
        };
        let file = std::fs::File::open(path).map_err(|_| error(ClientFileProblem::Unreadable))?;
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        file.take(MAX_CLIENT_FILE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error(ClientFileProblem::Unreadable))?;
        if bytes.len() as u64 > MAX_CLIENT_FILE {
            return Err(error(ClientFileProblem::TooLarge));
        }
        // The parsed file holds the client secret: its strings are wiped on drop.
        let value = WipedJson(
            serde_json::from_slice(&bytes).map_err(|_| error(ClientFileProblem::NotJson))?,
        );
        let installed = value
            .value()
            .as_object()
            .ok_or(error(ClientFileProblem::NotAnObject))?
            .get("installed")
            .and_then(serde_json::Value::as_object)
            .ok_or(error(ClientFileProblem::NoInstalledClient))?;
        let text = |name: &str| {
            installed
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
        };
        let client_id = text("client_id").ok_or(error(ClientFileProblem::MissingClientId))?;
        let client_secret =
            text("client_secret").ok_or(error(ClientFileProblem::MissingClientSecret))?;
        Ok(ClientConfig {
            client_id: client_id.to_owned(),
            client_secret: Secret::from(client_secret),
        })
    }

    /// The client id, sent in the authorization address and the token requests.
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The client secret, sent in the token and refresh requests (3.1, 3.4, 3.8).
    #[must_use]
    pub fn client_secret(&self) -> &Secret {
        &self.client_secret
    }
}

/// Prints neither value: the id is not secret, but nothing of the client file is formatted.
impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientConfig([redacted])")
    }
}

/// What is wrong with a client file. A fixed label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientFileProblem {
    /// The file could not be opened or read.
    Unreadable,
    /// The file is larger than [`MAX_CLIENT_FILE`].
    TooLarge,
    /// The file is not JSON.
    NotJson,
    /// The JSON is not an object.
    NotAnObject,
    /// There is no object `installed` (a Web or other client type, or another file).
    NoInstalledClient,
    /// `installed.client_id` is missing, empty or not a string.
    MissingClientId,
    /// `installed.client_secret` is missing, empty or not a string.
    MissingClientSecret,
}

impl ClientFileProblem {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ClientFileProblem::Unreadable => "cannot be read",
            ClientFileProblem::TooLarge => "is too large to be a client file",
            ClientFileProblem::NotJson => "is not JSON",
            ClientFileProblem::NotAnObject => "is not a JSON object",
            ClientFileProblem::NoInstalledClient => "holds no Desktop app client (`installed`)",
            ClientFileProblem::MissingClientId => "has no client id",
            ClientFileProblem::MissingClientSecret => "has no client secret",
        }
    }
}

impl fmt::Display for ClientFileProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ClientFileProblem {}

/// A client file that could not be used: its path and a fixed label, nothing of its content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientFileError {
    /// The path that was read.
    pub path: PathBuf,
    /// What is wrong.
    pub problem: ClientFileProblem,
}

impl fmt::Display for ClientFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the Google client file {} {}",
            self.path.display(),
            self.problem
        )
    }
}

impl std::error::Error for ClientFileError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A made-up client file in a directory of this test run (g48). No real client file is read.
    struct MadeUp {
        dir: PathBuf,
        path: PathBuf,
    }

    impl MadeUp {
        fn new(content: &[u8]) -> MadeUp {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "fabric-gc-client-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).expect("a temporary directory");
            let path = dir.join("client.json");
            std::fs::write(&path, content).expect("write the made-up file");
            MadeUp { dir, path }
        }
    }

    impl Drop for MadeUp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const SECRET: &str = "GOCSPX-madeUpSecretValue";
    const ID: &str = "1234-madeup.apps.googleusercontent.com";

    fn error_of(content: &str) -> ClientFileError {
        let file = MadeUp::new(content.as_bytes());
        let err = ClientConfig::from_file(&file.path).expect_err("refused");
        assert_eq!(err.path, file.path);
        let text = format!("{err} {err:?}");
        assert!(text.contains("client.json"), "names the path: {text}");
        for leaked in [SECRET, ID, "madeUp", "redirect", "installed\"", "{\""] {
            assert!(!text.contains(leaked), "leaks {leaked}: {text}");
        }
        err
    }

    #[test]
    fn the_installed_shape_is_read_and_other_members_are_ignored() {
        let file = MadeUp::new(
            format!(
                r#"{{"installed":{{"client_id":"{ID}","project_id":"p","auth_uri":"https://evil.example/auth",
                "token_uri":"https://evil.example/token","client_secret":"{SECRET}",
                "redirect_uris":["http://localhost"]}},"other":1}}"#
            )
            .as_bytes(),
        );
        let config = ClientConfig::from_file(&file.path).expect("read");
        assert_eq!(config.client_id(), ID);
        assert_eq!(config.client_secret().expose(), SECRET);
        assert_eq!(format!("{config:?}"), "ClientConfig([redacted])");
    }

    #[test]
    fn a_missing_secret_is_an_error_naming_the_path_and_no_content() {
        let err = error_of(&format!(r#"{{"installed":{{"client_id":"{ID}"}}}}"#));
        assert_eq!(err.problem, ClientFileProblem::MissingClientSecret);
        let err = error_of(&format!(
            r#"{{"installed":{{"client_id":"{ID}","client_secret":""}}}}"#
        ));
        assert_eq!(err.problem, ClientFileProblem::MissingClientSecret);
    }

    #[test]
    fn a_missing_client_id_is_an_error() {
        let err = error_of(&format!(
            r#"{{"installed":{{"client_secret":"{SECRET}"}}}}"#
        ));
        assert_eq!(err.problem, ClientFileProblem::MissingClientId);
        let err = error_of(&format!(
            r#"{{"installed":{{"client_id":7,"client_secret":"{SECRET}"}}}}"#
        ));
        assert_eq!(err.problem, ClientFileProblem::MissingClientId);
    }

    #[test]
    fn another_shape_is_an_error_naming_the_path_and_no_content() {
        let web = format!(r#"{{"web":{{"client_id":"{ID}","client_secret":"{SECRET}"}}}}"#);
        assert_eq!(error_of(&web).problem, ClientFileProblem::NoInstalledClient);
        let flat = format!(r#"{{"installed":"{SECRET}"}}"#);
        assert_eq!(
            error_of(&flat).problem,
            ClientFileProblem::NoInstalledClient
        );
        let array = format!(r#"["{SECRET}"]"#);
        assert_eq!(error_of(&array).problem, ClientFileProblem::NotAnObject);
        let broken = format!(r#"{{"installed":{{"client_secret":"{SECRET}""#);
        assert_eq!(error_of(&broken).problem, ClientFileProblem::NotJson);
    }

    #[test]
    fn a_missing_or_huge_file_is_an_error() {
        let file = MadeUp::new(b"{}");
        let missing = file.dir.join("absent.json");
        let err = ClientConfig::from_file(&missing).expect_err("refused");
        assert_eq!(err.problem, ClientFileProblem::Unreadable);
        assert!(err.to_string().contains("absent.json"));
        let huge = MadeUp::new(&vec![b' '; 70_000]);
        let err = ClientConfig::from_file(&huge.path).expect_err("refused");
        assert_eq!(err.problem, ClientFileProblem::TooLarge);
    }

    #[test]
    fn new_refuses_empty_values() {
        assert!(ClientConfig::new("id", Secret::from("s")).is_ok());
        assert_eq!(
            ClientConfig::new("", Secret::from("s")).err(),
            Some(ClientFileProblem::MissingClientId)
        );
        assert_eq!(
            ClientConfig::new("id", Secret::from("")).err(),
            Some(ClientFileProblem::MissingClientSecret)
        );
    }
}
