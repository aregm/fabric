//! One run of the broker child: start, relay, notice its death, stop.
//!
//! Threads and locks. A reader thread owns the child's stdout and the sink; it is the only
//! caller of the sink and calls it while holding no lock. The child's stdin, the child handle
//! and the session state each have their own mutex, and no code holds one of them while waiting
//! for another, except that the stdin mutex is only ever tried, never waited for, by the code
//! that ends the session (a `send` blocked on a full pipe must not keep `stop` from killing the
//! child).
//!
//! Who ends the session. The first of these claims the ending, under the state mutex:
//! `stop` (the ending is `Stopped`), the reader at a line it cannot validate
//! (`ProtocolViolation`), the reader at the end of the child's stdout (`Exited`). The reader
//! sends `Ended` once, after the child has been reaped and the child's stdout is closed, and
//! then drops the sink. So `Ended` is the sink's last event, whoever claimed it.

use std::fmt;
use std::io::{self, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use fabric_desktop_contracts::{Entry, Frame, MAX_FRAME_BYTES, Refusal, decode, parse_strict};

use crate::config::BrokerConfig;
use crate::line::{Line, LineReader};

/// How long the relay waits for the broker to exit after its stdin was closed or its stdout
/// ended, before it kills it. `Drop` uses it; the host passes it to [`Session::stop`].
pub const QUIT_DRAIN: Duration = Duration::from_secs(2);

/// How often a bounded wait asks whether the child has exited.
const POLL: Duration = Duration::from_millis(5);

/// What the relay hands to the sink.
#[allow(
    clippy::large_enum_variant,
    reason = "one event per frame, moved once into the sink; boxing would only complicate its use"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A frame the broker wrote that decoded as an owner response or an owner notice. The
    /// frame is `Frame::OwnerResponse` or `Frame::OwnerNotice`, never another kind.
    Frame(Frame),
    /// The session ended. It is the last event of a session and comes exactly once.
    Ended(Ending),
}

/// Why a session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The broker ended by itself or was killed from outside. `code` is its exit code when it
    /// has one; `description` says how it ended, for example "exit code 0" or (Unix) "killed by
    /// signal 9".
    Exited {
        code: Option<i32>,
        description: String,
    },
    /// The broker wrote something that is not a valid frame; the relay killed it.
    ProtocolViolation { reason: String },
    /// [`Session::stop`] was called (or the `Session` dropped); `killed` when the drain bound
    /// passed and the relay killed the broker.
    Stopped { killed: bool },
}

/// Why [`Session::send`] did not write a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The bytes are not a valid owner request frame; nothing was written.
    Refused(Refusal),
    /// The session has ended or is ending; nothing was written.
    NotRunning,
    /// Writing to the broker's stdin failed.
    Io(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Refused(refusal) => write!(f, "the request was refused: {refusal}"),
            SendError::NotRunning => write!(f, "the broker is not running"),
            SendError::Io(detail) => write!(f, "writing to the broker failed: {detail}"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }
}

/// Who claimed the end of the session.
#[derive(Debug)]
enum Claim {
    Running,
    Stopping,
    Exited,
    Violation,
}

struct State {
    claim: Claim,
    /// Set by `stop` once the child it claimed is reaped; the reader waits for it.
    stop_done: bool,
}

struct Shared {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    state: Mutex<State>,
    stop_done: Condvar,
    /// The relay sent the child a kill.
    killed: AtomicBool,
}

/// A running broker child and the reader thread that relays its output.
///
/// It is `Send` and `Sync`: `send` and `stop` may be called from any thread, including from
/// inside the sink.
pub struct Session {
    shared: Arc<Shared>,
    pid: u32,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("pid", &self.pid).finish()
    }
}

impl Session {
    /// Starts the broker with `config.command()` and a reader thread that hands every event to
    /// `sink`. The sink is called on the reader thread, one event at a time, and dropped after
    /// `Ended`.
    ///
    /// A limit on macOS: `std` makes a pipe and marks it close-on-exec in two steps, so a
    /// process that another thread starts at the same moment can inherit this broker's pipes.
    /// The session then sees the end of the broker's output only when that other process exits.
    /// The host starts one process at a time, so nothing here guards against it; code that
    /// starts several must not start them at the same moment.
    pub fn start<F>(config: &BrokerConfig, sink: F) -> io::Result<Session>
    where
        F: FnMut(Event) + Send + 'static,
    {
        let mut child = config.command().spawn()?;
        let pipes = (child.stdin.take(), child.stdout.take());
        let (Some(stdin), Some(stdout)) = pipes else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other(
                "the broker's stdin or stdout is not piped",
            ));
        };
        let pid = child.id();
        let shared = Arc::new(Shared {
            child: Mutex::new(child),
            stdin: Mutex::new(Some(stdin)),
            state: Mutex::new(State {
                claim: Claim::Running,
                stop_done: false,
            }),
            stop_done: Condvar::new(),
            killed: AtomicBool::new(false),
        });
        let reader_shared = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("fabric-desktop-relay".to_owned())
            .spawn(move || relay_output(&reader_shared, stdout, sink));
        if let Err(e) = spawned {
            let _ = shared.kill();
            return Err(e);
        }
        Ok(Session { shared, pid })
    }

    /// The broker's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Decodes `request` as an owner request frame and writes its canonical line to the broker.
    ///
    /// A refused frame is returned as `Refused` and nothing is written. After the session has
    /// ended, or while it is ending, `NotRunning`. A broken pipe (the broker is gone but the
    /// reader has not seen it yet) is also `NotRunning`.
    pub fn send(&self, request: &[u8]) -> Result<(), SendError> {
        let frame = decode(Entry::OwnerRequest, request).map_err(SendError::Refused)?;
        if !matches!(lock(&self.shared.state).claim, Claim::Running) {
            return Err(SendError::NotRunning);
        }
        let mut stdin = lock(&self.shared.stdin);
        let Some(pipe) = stdin.as_mut() else {
            return Err(SendError::NotRunning);
        };
        let line = frame.to_line();
        match pipe.write_all(line.as_bytes()).and_then(|()| pipe.flush()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Err(SendError::NotRunning),
            Err(e) => Err(SendError::Io(e.to_string())),
        }
    }

    /// Closes the broker's stdin, which the broker reads as the order to stop, waits up to
    /// `drain` for it to exit and kills it if it has not. Returns when the child is gone.
    ///
    /// Idempotent. When the session was still running, the sink gets `Ended(Stopped)`; when it
    /// was already ending for another reason, that reason stands. `Ended` is sent by the reader
    /// thread and may arrive after this returns.
    pub fn stop(&self, drain: Duration) {
        let claimed = {
            let mut state = lock(&self.shared.state);
            if matches!(state.claim, Claim::Running) {
                state.claim = Claim::Stopping;
                true
            } else {
                false
            }
        };
        // An error here means the child's status could not be read even after a kill; there is
        // nothing more the relay can do about it.
        let _ = self.shared.reap(drain);
        if claimed {
            lock(&self.shared.state).stop_done = true;
            self.shared.stop_done.notify_all();
        }
    }
}

impl Drop for Session {
    /// Stops the session with [`QUIT_DRAIN`], so a broker never outlives its `Session`.
    fn drop(&mut self) {
        self.stop(QUIT_DRAIN);
    }
}

impl Shared {
    /// Closes stdin, waits up to `drain` for the child to exit, then kills it.
    fn reap(&self, drain: Duration) -> io::Result<ExitStatus> {
        let deadline = Instant::now() + drain;
        let mut stdin_closed = false;
        loop {
            if !stdin_closed {
                stdin_closed = self.try_close_stdin();
            }
            match lock(&self.child).try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(_) => break,
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            thread::sleep(POLL.min(deadline - now));
        }
        self.kill()
    }

    /// Kills the child unless it has exited, and reaps it.
    fn kill(&self) -> io::Result<ExitStatus> {
        let status = {
            let mut child = lock(&self.child);
            if let Ok(Some(status)) = child.try_wait() {
                return Ok(status);
            }
            self.killed.store(true, Ordering::SeqCst);
            child.kill()?;
            child.wait()?
        };
        // A `send` blocked on a full pipe has failed by now, so this lock is free soon.
        lock(&self.stdin).take();
        Ok(status)
    }

    /// Drops the child's stdin unless a `send` is writing to it right now.
    fn try_close_stdin(&self) -> bool {
        match self.stdin.try_lock() {
            Ok(mut stdin) => {
                stdin.take();
                true
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().take();
                true
            }
            Err(TryLockError::WouldBlock) => false,
        }
    }

    /// Claims the end of the session for the reader, unless `stop` has claimed it.
    fn claim_for_reader(&self, claim: Claim) -> bool {
        let mut state = lock(&self.state);
        if matches!(state.claim, Claim::Running) {
            state.claim = claim;
            true
        } else {
            false
        }
    }

    fn wait_for_stop(&self) {
        let mut state = lock(&self.state);
        while !state.stop_done {
            state = self
                .stop_done
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// The reader thread: relays the child's output to the sink until the session ends, then
/// sends `Ended` once.
fn relay_output<F: FnMut(Event)>(shared: &Shared, stdout: ChildStdout, mut sink: F) {
    let mut lines = LineReader::new(BufReader::new(stdout), MAX_FRAME_BYTES);
    let violation = loop {
        match lines.next_line() {
            Ok(Line::Complete(bytes)) => match decode_output(&bytes) {
                Ok(frame) => sink(Event::Frame(frame)),
                Err(refusal) => {
                    break Some(format!("the broker wrote an invalid frame: {refusal}"));
                }
            },
            Ok(Line::Oversized) => {
                break Some(format!(
                    "the broker wrote a line longer than {MAX_FRAME_BYTES} bytes"
                ));
            }
            Ok(Line::Unterminated) => {
                break Some("the broker's output ended inside a line".to_owned());
            }
            // A read error is treated as the end of the output: nothing more can be read.
            Ok(Line::End) | Err(_) => break None,
        }
    };
    let ending = match violation {
        Some(reason) if shared.claim_for_reader(Claim::Violation) => {
            let _ = shared.kill();
            Ending::ProtocolViolation { reason }
        }
        None if shared.claim_for_reader(Claim::Exited) => {
            let result = shared.reap(QUIT_DRAIN);
            exited(result, shared.killed.load(Ordering::SeqCst))
        }
        _ => {
            // `stop` claimed the ending. Nothing more is forwarded; the rest of the output is
            // read and dropped, so the broker never blocks on a full pipe while it drains.
            let mut rest = lines.into_input();
            let _ = io::copy(&mut rest, &mut io::sink());
            shared.wait_for_stop();
            Ending::Stopped {
                killed: shared.killed.load(Ordering::SeqCst),
            }
        }
    };
    sink(Event::Ended(ending));
}

/// Reads one line of the broker's output: a frame with a `sequence` member as an owner
/// notice, any other as an owner response (the two entries a broker writes on the owner
/// channel). Only the member's presence is looked at; `decode` reads the frame.
fn decode_output(bytes: &[u8]) -> Result<Frame, Refusal> {
    let entry = match parse_strict(bytes) {
        Ok(value) if value.get("sequence").is_some() => Entry::OwnerNotice,
        _ => Entry::OwnerResponse,
    };
    decode(entry, bytes)
}

fn exited(result: io::Result<ExitStatus>, killed: bool) -> Ending {
    let (code, mut description) = match result {
        Ok(status) => describe(status),
        Err(e) => (None, format!("its exit status could not be read: {e}")),
    };
    if killed {
        description.push_str("; the relay killed it after its output closed");
    }
    Ending::Exited { code, description }
}

fn describe(status: ExitStatus) -> (Option<i32>, String) {
    if let Some(code) = status.code() {
        return (Some(code), format!("exit code {code}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (None, format!("killed by signal {signal}"));
        }
    }
    (None, format!("ended with {status}"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_with_a_sequence_is_read_as_a_notice() {
        let line = br#"{"change":{"kind":"snapshot_required"},"protocol":"fabric-desktop/1","sequence":3}"#;
        assert!(matches!(decode_output(line), Ok(Frame::OwnerNotice(_))));
    }

    #[test]
    fn a_frame_without_a_sequence_is_read_as_a_response() {
        let line = br#"{"error":{"code":"internal","correlation_id":null,"detail":null,"message":"Something went wrong in the broker.","recovery":"report_problem","retryable":false},"id":"snapshot-3","method":"get_snapshot","protocol":"fabric-desktop/1"}"#;
        assert!(matches!(decode_output(line), Ok(Frame::OwnerResponse(_))));
    }

    #[test]
    fn a_request_is_not_accepted_as_output() {
        let line = br#"{"id":"snapshot-1","method":"get_snapshot","params":{},"protocol":"fabric-desktop/1"}"#;
        assert!(decode_output(line).is_err());
    }

    #[test]
    fn garbage_is_refused_at_the_frame_layer() {
        let refusal = decode_output(b"not json").unwrap_err();
        assert_eq!(refusal.layer, fabric_desktop_contracts::Layer::Frame);
    }

    #[test]
    fn statuses_are_described() {
        let ending = exited(Err(io::Error::other("gone")), false);
        assert_eq!(
            ending,
            Ending::Exited {
                code: None,
                description: "its exit status could not be read: gone".to_owned()
            }
        );
    }

    #[test]
    fn send_errors_read_plainly() {
        assert_eq!(
            SendError::NotRunning.to_string(),
            "the broker is not running"
        );
    }
}
