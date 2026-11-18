//! The agent socket: its path, the checks before connecting, the connection and the handshake
//! (W06 4.1, 4.2, 4.4). Unix only.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fabric_desktop_contracts::error::{AgentError, AgentFault, UnsupportedSchemaVersion};
use fabric_desktop_contracts::scalar::{Count, Id, PositiveCount};
use fabric_desktop_contracts::{MAX_FRAME_BYTES, SCHEMA_VERSION};

use super::frames::{Line, LineReader};
use super::handshake::{self, HelloAnswer, Malformed, ProveAnswer};
use super::key::KeyError;
use super::key::file::FileKeyStore;
use crate::bridge::{ConnectOutcome, not_running};
use crate::mcp::render::{BridgeFault, bridge_error};

/// The installed broker's data directory under `HOME` (W06 4.1; W04 D28).
pub const INSTALLED_DATA_DIR: &str =
    "Library/Application Support/ai.paracels.fabric.calendar/broker";
/// The socket's name in the data directory's `run` directory.
pub const SOCKET_NAME: &str = "agent.sock";
/// Connecting and the handshake together (W06 2.5).
pub const CONNECT_LIMIT: Duration = Duration::from_secs(5);

/// Why there is no connection. Each names the check that failed, in fixed words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketError {
    NoHome,
    NoName,
    DirectoryMissing,
    NotDirectory,
    DirectoryMode,
    DirectoryOwner,
    SocketMissing,
    NotSocket,
    SocketMode,
    SocketOwner,
    Refused,
}

impl SocketError {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SocketError::NoHome => "HOME is not set to an absolute path",
            SocketError::NoName => "the socket path has no directory or name",
            SocketError::DirectoryMissing => "the socket's directory cannot be entered",
            SocketError::NotDirectory => "the socket's directory is not a directory",
            SocketError::DirectoryMode => "the socket's directory's mode is not 0700",
            SocketError::DirectoryOwner => "the socket's directory belongs to another user",
            SocketError::SocketMissing => "there is no socket",
            SocketError::NotSocket => "agent.sock is not a socket",
            SocketError::SocketMode => "the socket's mode is not 0600",
            SocketError::SocketOwner => "the socket belongs to another user",
            SocketError::Refused => "the connection was refused",
        }
    }
}

/// The installed socket, `$HOME/<INSTALLED_DATA_DIR>/run/agent.sock`.
pub fn installed_socket_path(home: Option<&OsStr>) -> Result<PathBuf, SocketError> {
    let home = home
        .map(Path::new)
        .filter(|h| h.is_absolute())
        .ok_or(SocketError::NoHome)?;
    Ok(home.join(INSTALLED_DATA_DIR).join("run").join(SOCKET_NAME))
}

fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Makes the socket's directory the working directory, checks it and the socket in it, and
/// connects by the relative name (W06 4.2).
pub fn check_and_connect(path: &Path) -> Result<UnixStream, SocketError> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or(SocketError::NoName)?;
    let name = path.file_name().ok_or(SocketError::NoName)?;
    std::env::set_current_dir(dir).map_err(|_| SocketError::DirectoryMissing)?;
    let here = fs::symlink_metadata(".").map_err(|_| SocketError::DirectoryMissing)?;
    if !here.file_type().is_dir() {
        return Err(SocketError::NotDirectory);
    }
    if here.mode() & 0o7777 != 0o700 {
        return Err(SocketError::DirectoryMode);
    }
    if here.uid() != euid() {
        return Err(SocketError::DirectoryOwner);
    }
    let socket = fs::symlink_metadata(name).map_err(|_| SocketError::SocketMissing)?;
    if !socket.file_type().is_socket() {
        return Err(SocketError::NotSocket);
    }
    if socket.mode() & 0o7777 != 0o600 {
        return Err(SocketError::SocketMode);
    }
    if socket.uid() != euid() {
        return Err(SocketError::SocketOwner);
    }
    UnixStream::connect(name).map_err(|_| SocketError::Refused)
}

/// One write to the broker, a whole frame (W06 2.5).
pub const WRITE_LIMIT: Duration = Duration::from_secs(5);

/// The socket's read side, with an optional deadline. While the deadline is set, every read
/// gets only the time that is left of it, so a peer that sends a byte now and then cannot
/// stretch the limit; once it has passed, a read fails with `TimedOut`.
pub struct TimedReader {
    stream: UnixStream,
    deadline: Option<Instant>,
}

impl TimedReader {
    #[must_use]
    pub fn new(stream: UnixStream, deadline: Option<Instant>) -> TimedReader {
        TimedReader { stream, deadline }
    }

    /// Removes the deadline: reads then wait as long as it takes.
    pub fn clear_deadline(&mut self) -> io::Result<()> {
        self.deadline = None;
        self.stream.set_read_timeout(None)
    }
}

impl Read for TimedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            self.stream.set_read_timeout(Some(left))?;
        }
        self.stream.read(buf)
    }
}

/// Writes `bytes` whole before `deadline`, re-arming the write timeout with the time left
/// before every write. A timeout, an error or a peer that takes nothing fails the write; the
/// caller then drops the connection, as a frame may be half written (W06 2.5).
pub fn write_before(stream: &mut UnixStream, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_write_timeout(Some(left))?;
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => written += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// An approved connection: the reader, with whatever it has buffered, and the writer.
pub struct Connection {
    pub reader: LineReader<BufReader<TimedReader>>,
    pub writer: UnixStream,
}

/// Everything a connection needs: the agent's id, the socket's path and the key store.
#[derive(Clone, Debug)]
pub struct Connector {
    pub enrollment: Id,
    pub socket: Result<PathBuf, SocketError>,
    /// `None` in a build with no key store (W06 5.4).
    pub keys: Option<FileKeyStore>,
}

fn refused(error: AgentError, code: &str, reason: &'static str) -> ConnectOutcome {
    ConnectOutcome::Refused {
        error,
        pairing_code: Some(code.to_owned()),
        reason,
    }
}

fn not_running_with(code: &str, reason: &'static str) -> ConnectOutcome {
    refused(
        bridge_error(
            AgentFault::BrokerNotRunning,
            "Fabric Calendar is not running.",
        ),
        code,
        reason,
    )
}

fn fault(fault: BridgeFault, code: Option<&str>, reason: &'static str) -> ConnectOutcome {
    ConnectOutcome::Fault {
        fault,
        pairing_code: code.map(str::to_owned),
        reason,
    }
}

/// How the handshake failed on the wire.
enum Wire {
    OutOfTime,
    Closed,
    Malformed,
}

impl Connector {
    /// Gets the key, connects and runs the handshake, all within [`CONNECT_LIMIT`].
    #[must_use]
    pub fn connect(&self) -> (ConnectOutcome, Option<Connection>) {
        let deadline = Instant::now() + CONNECT_LIMIT;
        let Some(keys) = &self.keys else {
            return (not_running(KeyError::NoStore.as_str()), None);
        };
        let key = match keys.load_or_create(&self.enrollment) {
            Ok(key) => key,
            Err(e) => return (fault(BridgeFault::Key, None, e.as_str()), None),
        };
        let code = key.pairing_code();
        let path = match &self.socket {
            Ok(path) => path,
            Err(e) => return (not_running_with(&code, e.as_str()), None),
        };
        // `connect` itself has no time limit. On Linux a listener with a full backlog can
        // block it; the product runs on macOS, where a full backlog refuses at once.
        let stream = match check_and_connect(path) {
            Ok(stream) => stream,
            Err(e) => return (not_running_with(&code, e.as_str()), None),
        };
        let Ok(mut writer) = stream.try_clone() else {
            return (
                not_running_with(&code, "the socket could not be shared"),
                None,
            );
        };
        let mut reader = LineReader::new(
            BufReader::new(TimedReader::new(stream, Some(deadline))),
            MAX_FRAME_BYTES,
        );
        let wire = |w: Wire, code: &str| match w {
            Wire::OutOfTime => not_running_with(code, "the handshake ran out of time"),
            Wire::Closed => not_running_with(code, "the connection closed during the handshake"),
            Wire::Malformed => fault(BridgeFault::HandshakeAnswer, Some(code), Malformed::REASON),
        };

        let Some(hello_random) = crate::random_128() else {
            return (
                fault(
                    BridgeFault::RandomSource,
                    Some(&code),
                    "the random source failed",
                ),
                None,
            );
        };
        let hello_id = format!("hello-{}", crate::hex(&hello_random));
        let public = key.public_key_text();
        let answer = exchange(
            &mut writer,
            &mut reader,
            &handshake::hello_line(&hello_id, &self.enrollment, &public),
            deadline,
        );
        let nonce = match answer.map(|bytes| handshake::read_hello_answer(&bytes, &hello_id)) {
            Err(w) => return (wire(w, &code), None),
            Ok(Err(Malformed)) => return (wire(Wire::Malformed, &code), None),
            Ok(Ok(HelloAnswer::SchemaVersion(received))) => {
                return (schema_version_error(received, &code), None);
            }
            Ok(Ok(HelloAnswer::Error(error))) => {
                return (refused(error, &code, "the broker refused the hello"), None);
            }
            Ok(Ok(HelloAnswer::Nonce(nonce))) => nonce,
        };

        let Some(prove_random) = crate::random_128() else {
            return (
                fault(
                    BridgeFault::RandomSource,
                    Some(&code),
                    "the random source failed",
                ),
                None,
            );
        };
        let prove_id = format!("prove-{}", crate::hex(&prove_random));
        let signature = handshake::signature(&key, &nonce, &self.enrollment);
        let answer = exchange(
            &mut writer,
            &mut reader,
            &handshake::prove_line(&prove_id, &signature),
            deadline,
        );
        match answer.map(|bytes| handshake::read_prove_answer(&bytes, &prove_id)) {
            Err(w) => (wire(w, &code), None),
            Ok(Err(Malformed)) => (wire(Wire::Malformed, &code), None),
            Ok(Ok(ProveAnswer::Error(error))) => {
                (refused(error, &code, "the broker refused the proof"), None)
            }
            Ok(Ok(ProveAnswer::Pending)) => (ConnectOutcome::Pending { pairing_code: code }, None),
            Ok(Ok(ProveAnswer::Approved)) => {
                if reader.get_mut().get_mut().clear_deadline().is_err() {
                    return (
                        not_running_with(&code, "the socket could not be set up"),
                        None,
                    );
                }
                (
                    ConnectOutcome::Approved { pairing_code: code },
                    Some(Connection { reader, writer }),
                )
            }
        }
    }
}

fn schema_version_error(received: u64, code: &str) -> ConnectOutcome {
    match Count::new(received) {
        Ok(received_count) => refused(
            bridge_error(
                AgentFault::UnsupportedSchemaVersion(UnsupportedSchemaVersion {
                    received: received_count,
                    supported: PositiveCount::of(SCHEMA_VERSION),
                }),
                "The broker speaks another schema version.",
            ),
            code,
            "the broker's schema version differs",
        ),
        Err(_) => fault(BridgeFault::HandshakeAnswer, Some(code), Malformed::REASON),
    }
}

/// Writes one handshake line and reads one answer line, both before the one `deadline`.
fn exchange(
    writer: &mut UnixStream,
    reader: &mut LineReader<BufReader<TimedReader>>,
    line: &str,
    deadline: Instant,
) -> Result<Vec<u8>, Wire> {
    let io_error = |e: io::Error| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Wire::OutOfTime,
        _ => Wire::Closed,
    };
    write_before(writer, line.as_bytes(), deadline).map_err(io_error)?;
    match reader.next_line().map_err(io_error)? {
        Line::Line(bytes) => Ok(bytes),
        Line::TooLong => Err(Wire::Malformed),
        Line::End => Err(Wire::Closed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    /// The tests that change the working directory run one at a time.
    static CWD: Mutex<()> = Mutex::new(());

    // The connect-and-handshake limit is one total: a peer that sends a byte now and then
    // does not stretch it (W06 2.5).
    #[test]
    fn handshake_read_deadline_is_total() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let drip = std::thread::spawn(move || {
            let mut theirs = theirs;
            for _ in 0..20 {
                if theirs.write_all(b"x").is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let start = Instant::now();
        let deadline = start + Duration::from_millis(200);
        let mut reader = LineReader::new(
            BufReader::new(TimedReader::new(ours, Some(deadline))),
            MAX_FRAME_BYTES,
        );
        let error = reader.next_line().unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(start.elapsed() < Duration::from_millis(800));
        drop(reader);
        drip.join().unwrap();
    }

    // One write to the broker is bounded: a peer that does not read fails it in time.
    #[test]
    fn write_is_bounded() {
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        let start = Instant::now();
        let big = vec![b'x'; 8 * 1024 * 1024];
        let result = write_before(&mut ours, &big, start + Duration::from_millis(200));
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(1500));
        drop(theirs);
        let mut ok_pair = UnixStream::pair().unwrap();
        assert!(write_before(&mut ok_pair.0, b"line\n", Instant::now() + WRITE_LIMIT).is_ok());
        let mut got = [0u8; 5];
        ok_pair.1.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"line\n");
    }

    // s01: the path from HOME; no HOME is a refusal.
    #[test]
    fn s01_installed_path() {
        assert_eq!(
            installed_socket_path(Some(OsStr::new("/Users/someone"))),
            Ok(PathBuf::from(
                "/Users/someone/Library/Application Support/ai.paracels.fabric.calendar/broker/run/agent.sock"
            ))
        );
        assert_eq!(installed_socket_path(None), Err(SocketError::NoHome));
        assert_eq!(
            installed_socket_path(Some(OsStr::new(""))),
            Err(SocketError::NoHome)
        );
        assert_eq!(
            installed_socket_path(Some(OsStr::new("relative"))),
            Err(SocketError::NoHome)
        );
    }

    // s02: a directory not 0700, a socket not 0600, a link in place of the socket and a plain
    // file in its place are refused.
    #[test]
    fn s02_checks() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = std::env::current_dir().ok();
        let root = std::env::temp_dir().join(format!(
            "fmb-s02-{}",
            crate::hex(&crate::random_128().unwrap()[..4])
        ));
        let run = root.join("run");
        fs::create_dir_all(&run).unwrap();
        let mode =
            |p: &Path, m: u32| fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
        mode(&run, 0o700);
        // Bind by the relative name, as the broker is asked to (LR-B2).
        std::env::set_current_dir(&run).unwrap();
        let listener = UnixListener::bind(SOCKET_NAME).unwrap();
        let socket = run.join(SOCKET_NAME);
        mode(&socket, 0o600);

        assert!(check_and_connect(&socket).is_ok());
        assert_eq!(
            std::env::current_dir().unwrap().canonicalize().unwrap(),
            run.canonicalize().unwrap()
        );

        mode(&run, 0o755);
        assert_eq!(
            check_and_connect(&socket).err(),
            Some(SocketError::DirectoryMode)
        );
        mode(&run, 0o700);

        mode(&socket, 0o666);
        assert_eq!(
            check_and_connect(&socket).err(),
            Some(SocketError::SocketMode)
        );
        mode(&socket, 0o600);

        // A link in place of the socket, pointing at a real socket elsewhere.
        let elsewhere = root.join("real.sock");
        fs::rename(&socket, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &socket).unwrap();
        assert_eq!(
            check_and_connect(&socket).err(),
            Some(SocketError::NotSocket)
        );
        fs::remove_file(&socket).unwrap();

        fs::write(&socket, b"").unwrap();
        mode(&socket, 0o600);
        assert_eq!(
            check_and_connect(&socket).err(),
            Some(SocketError::NotSocket)
        );
        fs::remove_file(&socket).unwrap();

        assert_eq!(
            check_and_connect(&socket).err(),
            Some(SocketError::SocketMissing)
        );
        assert_eq!(
            check_and_connect(&root.join("nowhere").join(SOCKET_NAME)).err(),
            Some(SocketError::DirectoryMissing)
        );

        drop(listener);
        if let Some(before) = before {
            let _ = std::env::set_current_dir(before);
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
