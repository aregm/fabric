//! The host's state: one broker child at most, the connection state, and the webview's channel.
//!
//! Nothing here is Tauri, so it is tested without a window. The host relays and nothing more:
//! it never builds a frame, never answers for the broker and never looks at what a request
//! asks for. It holds no Google credential and no database.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use fabric_desktop_contracts::owner::OwnerResponse;
use fabric_desktop_contracts::{Entry, Frame, decode};
use fabric_desktop_relay::{
    BrokerConfig, ENVIRONMENT_ALLOWLIST, Event, InstanceError, InstanceLock, QUIT_DRAIN, Session,
};

use crate::bridge::{DisconnectReason, HostMessage, HostState, SendRefusal};

/// Where the host's messages go: the channel of the attached webview.
pub type Sink = Box<dyn Fn(HostMessage) + Send + Sync>;

/// Writes one line per event to stderr when the environment variable `FABRIC_DESKTOP_TRACE` is
/// `1`. A line holds methods, ids, sequences and sizes, never the content of a frame.
#[derive(Clone, Copy)]
pub struct Trace {
    enabled: bool,
    started: Instant,
}

impl Trace {
    #[must_use]
    pub fn from_environment() -> Trace {
        Trace::new(std::env::var_os("FABRIC_DESKTOP_TRACE").is_some_and(|value| value == "1"))
    }

    #[must_use]
    pub fn new(enabled: bool) -> Trace {
        Trace {
            enabled,
            started: Instant::now(),
        }
    }

    fn line(&self, text: impl FnOnce() -> String) {
        if self.enabled {
            eprintln!(
                "fabric-desktop +{:04}ms {}",
                self.started.elapsed().as_millis(),
                text()
            );
        }
    }
}

struct Inner {
    /// Counts broker starts: 0 before any, 1 for the first.
    run: u64,
    state: HostState,
    session: Option<Arc<Session>>,
    sink: Option<Sink>,
    /// Held for the life of the process once acquired.
    lock: Option<InstanceLock>,
    broker: PathBuf,
}

impl Inner {
    /// Records the state and tells the webview, in that order, under the host's lock, so the
    /// webview sees states and frames in the order they happened.
    fn set_state(&mut self, state: HostState, trace: Trace, extra: &str) {
        trace.line(|| format!("state run={} {}{extra}", self.run, describe_state(&state)));
        self.state = state;
        self.tell(HostMessage::State {
            run: self.run,
            state: self.state.clone(),
        });
    }

    fn tell(&self, message: HostMessage) {
        if let Some(sink) = &self.sink {
            sink(message);
        }
    }
}

/// The host. It is shared between the Tauri commands and the relay's reader thread.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Mutex<Inner>>,
    trace: Trace,
}

impl Host {
    #[must_use]
    pub fn new(trace: Trace) -> Host {
        Host {
            inner: Arc::new(Mutex::new(Inner {
                run: 0,
                state: HostState::ConfigurationError {
                    detail: "the application has not started the broker yet".to_owned(),
                },
                session: None,
                sink: None,
                lock: None,
                broker: PathBuf::new(),
            })),
            trace,
        }
    }

    /// Takes the single-instance lock in `lock_directory` and, if this is the only instance,
    /// starts the broker at `broker`. A second instance ends in the `conflict` state and
    /// starts nothing.
    pub fn start(&self, lock_directory: &Path, broker: &Path) {
        let mut inner = self.lock();
        inner.broker = broker.to_path_buf();
        match InstanceLock::acquire(lock_directory) {
            Ok(lock) => inner.lock = Some(lock),
            Err(InstanceError::Held) => {
                let detail =
                    "Fabric Calendar is already running. This window started no broker.".to_owned();
                inner.set_state(HostState::Conflict { detail }, self.trace, "");
                return;
            }
            Err(error) => {
                // Without the lock the host cannot know it is the only instance, so it does
                // not start a broker.
                let detail = format!("the single-instance lock could not be taken: {error}");
                inner.set_state(HostState::ConfigurationError { detail }, self.trace, "");
                return;
            }
        }
        self.start_broker(&mut inner);
    }

    /// Checks the configuration and then starts one broker child. The check comes first and
    /// starts nothing (lesson DEF-2 of the demo).
    fn start_broker(&self, inner: &mut Inner) {
        let config =
            match BrokerConfig::check(&inner.broker, ENVIRONMENT_ALLOWLIST, std::env::vars_os()) {
                Ok(config) => config,
                Err(error) => {
                    let detail = error.to_string();
                    inner.set_state(HostState::ConfigurationError { detail }, self.trace, "");
                    return;
                }
            };
        inner.run += 1;
        let run = inner.run;
        let host = self.clone();
        match Session::start(&config, move |event| host.relay(run, event)) {
            Ok(session) => {
                let extra = format!(
                    " pid={} broker={:?}",
                    session.pid(),
                    config.program().display().to_string()
                );
                inner.session = Some(Arc::new(session));
                inner.set_state(HostState::Connected, self.trace, &extra);
            }
            Err(error) => {
                let state = HostState::Disconnected {
                    reason: DisconnectReason::StartFailed,
                    detail: format!("the broker could not be started: {error}"),
                };
                inner.set_state(state, self.trace, "");
            }
        }
    }

    /// Called on the relay's reader thread for every event of the session of `run`.
    fn relay(&self, run: u64, event: Event) {
        let mut inner = self.lock();
        if inner.run != run {
            return;
        }
        match event {
            Event::Frame(frame) => {
                self.trace.line(|| describe_frame(&frame));
                inner.tell(HostMessage::Frame {
                    run,
                    frame: frame.to_value(),
                });
            }
            Event::Ended(ending) => {
                inner.session = None;
                inner.set_state(HostState::ended(&ending), self.trace, "");
            }
        }
    }

    /// `owner_attach`: the webview gives its channel and gets the current state at once.
    pub fn attach(&self, sink: Sink) {
        let mut inner = self.lock();
        self.trace.line(|| "attached".to_owned());
        inner.sink = Some(sink);
        inner.tell(HostMessage::State {
            run: inner.run,
            state: inner.state.clone(),
        });
    }

    /// `owner_send`: validates one owner request frame and writes it to the broker.
    pub fn send(&self, frame: &[u8]) -> Result<(), SendRefusal> {
        // The write happens outside the host's lock: a broker that is slow to read must not
        // keep the reader thread from delivering what the broker already wrote.
        let session = self.lock().session.clone();
        let Some(session) = session else {
            self.trace.line(|| "refused not_connected".to_owned());
            return Err(SendRefusal::NotConnected);
        };
        match session.send(frame) {
            Ok(()) => {
                self.trace.line(|| describe_request(frame));
                Ok(())
            }
            Err(error) => {
                let refusal = SendRefusal::from(error);
                self.trace.line(|| match &refusal {
                    SendRefusal::Refused { layer, code, .. } => {
                        format!("refused layer={layer} code={code}")
                    }
                    SendRefusal::NotConnected => "refused not_connected".to_owned(),
                    SendRefusal::Io { detail } => format!("refused io detail={detail:?}"),
                });
                Err(refusal)
            }
        }
    }

    /// `broker_restart`: the owner starts the broker again. Allowed only while disconnected.
    pub fn restart(&self) -> Result<(), String> {
        let mut inner = self.lock();
        if !matches!(inner.state, HostState::Disconnected { .. }) {
            return Err("The broker can be restarted only while it is disconnected.".to_owned());
        }
        self.start_broker(&mut inner);
        Ok(())
    }

    /// Quit: closes the broker's input, waits up to `QUIT_DRAIN` for it to end and kills it if
    /// it has not.
    pub fn quit(&self) {
        let session = self.lock().session.clone();
        if let Some(session) = session {
            self.trace.line(|| "quit: stopping the broker".to_owned());
            session.stop(QUIT_DRAIN);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn describe_state(state: &HostState) -> String {
    match state {
        HostState::Connected => "connected".to_owned(),
        HostState::Disconnected { reason, detail } => {
            format!("disconnected reason={} detail={detail:?}", reason.as_str())
        }
        HostState::Conflict { detail } => format!("conflict detail={detail:?}"),
        HostState::ConfigurationError { detail } => {
            format!("configuration_error detail={detail:?}")
        }
    }
}

/// The trace line of a request the host wrote to the broker. The frame decoded a moment ago
/// in `Session::send`, so it decodes here.
fn describe_request(bytes: &[u8]) -> String {
    match decode(Entry::OwnerRequest, bytes) {
        Ok(Frame::OwnerRequest(request)) => format!(
            "-> {} id={} bytes={}",
            request.call.method().as_str(),
            request.id.as_str(),
            bytes.len()
        ),
        _ => format!("-> ? bytes={}", bytes.len()),
    }
}

fn describe_frame(frame: &Frame) -> String {
    let bytes = frame.to_line().len() - 1;
    match frame {
        Frame::OwnerResponse(OwnerResponse::Result { id, result }) => format!(
            "<- response {} id={} result bytes={bytes}",
            result.method().as_str(),
            id.as_str()
        ),
        Frame::OwnerResponse(OwnerResponse::Error { id, method, error }) => format!(
            "<- response {} id={} error={} bytes={bytes}",
            method.as_ref().map_or("-", |m| m.as_str()),
            id.as_ref().map_or("-", |i| i.as_str()),
            error.code().as_str()
        ),
        Frame::OwnerNotice(notice) => format!(
            "<- notice sequence={} change={} bytes={bytes}",
            notice.sequence.get(),
            notice.change.kind()
        ),
        // The relay hands over owner responses and owner notices only.
        _ => format!("<- ? bytes={bytes}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    /// The broker that `scripts/verify-desktop.sh` and `scripts/build-desktop.sh` stage for the
    /// bundle. The tests that need a real broker use it.
    const STAGED_BROKER: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/binaries/fabric-broker-aarch64-apple-darwin"
    );

    const HELLO: &[u8] = br#"{"protocol":"fabric-desktop/1","id":"hello-1","method":"hello","params":{"client":{"name":"host test","build":"0"},"schema_version":1}}"#;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "fabric-desktop-host-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn attached(host: &Host) -> Receiver<HostMessage> {
        let (sender, receiver) = channel();
        let sender = Mutex::new(sender);
        host.attach(Box::new(move |message| {
            let _ = sender
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(message);
        }));
        receiver
    }

    fn next(receiver: &Receiver<HostMessage>) -> HostMessage {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the host sent a message in time")
    }

    fn state_of(message: HostMessage) -> (u64, HostState) {
        match message {
            HostMessage::State { run, state } => (run, state),
            other => panic!("expected a state, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_broker_is_a_configuration_error_and_nothing_starts() {
        let host = Host::new(Trace::new(false));
        let lock_directory = directory("missing");
        host.start(&lock_directory, Path::new("/nonexistent/fabric-broker"));
        let receiver = attached(&host);
        let (run, state) = state_of(next(&receiver));
        assert_eq!(run, 0, "no broker start was counted");
        assert!(matches!(state, HostState::ConfigurationError { .. }));
        assert_eq!(host.send(HELLO), Err(SendRefusal::NotConnected));
        assert!(
            host.restart().is_err(),
            "a configuration error has no restart"
        );
    }

    #[test]
    fn a_second_instance_is_a_conflict_and_starts_no_broker() {
        let lock_directory = directory("conflict");
        let first = InstanceLock::acquire(&lock_directory).expect("the first lock");
        let host = Host::new(Trace::new(false));
        host.start(&lock_directory, Path::new(STAGED_BROKER));
        let receiver = attached(&host);
        let (run, state) = state_of(next(&receiver));
        assert_eq!(run, 0);
        assert!(matches!(state, HostState::Conflict { .. }));
        assert_eq!(host.send(HELLO), Err(SendRefusal::NotConnected));
        assert!(host.restart().is_err(), "a conflict has no restart");
        drop(first);
    }

    #[test]
    fn a_relative_broker_path_is_refused_before_anything_starts() {
        let host = Host::new(Trace::new(false));
        host.start(&directory("relative"), Path::new("fabric-broker"));
        let (run, state) = state_of(next(&attached(&host)));
        assert_eq!(run, 0);
        assert!(matches!(state, HostState::ConfigurationError { .. }));
    }

    #[test]
    fn round_trip_death_and_restart_with_the_real_broker() {
        if !Path::new(STAGED_BROKER).is_file() {
            panic!(
                "the staged broker is missing at {STAGED_BROKER}; run sh scripts/verify-desktop.sh, which stages it"
            );
        }
        let host = Host::new(Trace::new(false));
        host.start(&directory("round-trip"), Path::new(STAGED_BROKER));
        let receiver = attached(&host);
        assert_eq!(state_of(next(&receiver)), (1, HostState::Connected));

        // A frame the contract refuses is not written, and the host says why.
        assert!(matches!(
            host.send(b"{}"),
            Err(SendRefusal::Refused {
                layer: "protocol",
                ..
            })
        ));
        assert!(host.restart().is_err(), "a running broker is not restarted");

        // Echo: the broker's answer comes back as a frame of run 1.
        host.send(HELLO).expect("hello is a valid frame");
        match next(&receiver) {
            HostMessage::Frame { run, frame } => {
                assert_eq!(run, 1);
                assert_eq!(frame["id"], "hello-1");
                assert_eq!(frame["method"], "hello");
                assert_eq!(frame["result"]["schema_version"], 1);
            }
            other => panic!("expected the hello answer, got {other:?}"),
        }

        // The broker dies: the host says so and starts nothing by itself.
        let pid = host.lock().session.as_ref().expect("a session").pid();
        let killed = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .expect("kill ran");
        assert!(killed.success());
        let (run, state) = state_of(next(&receiver));
        assert_eq!(run, 1);
        assert!(
            matches!(
                state,
                HostState::Disconnected {
                    reason: DisconnectReason::Exited,
                    ..
                }
            ),
            "{state:?}"
        );
        assert_eq!(host.send(HELLO), Err(SendRefusal::NotConnected));
        assert!(
            receiver.recv_timeout(Duration::from_millis(500)).is_err(),
            "nothing restarts without the owner"
        );

        // The owner restarts: a new run, and the round trip works again.
        host.restart()
            .expect("a disconnected broker can be restarted");
        assert_eq!(state_of(next(&receiver)), (2, HostState::Connected));
        host.send(HELLO).expect("hello is a valid frame");
        assert!(matches!(next(&receiver), HostMessage::Frame { run: 2, .. }));

        // Quit: the broker stops when its input closes.
        host.quit();
        let (run, state) = state_of(next(&receiver));
        assert_eq!(run, 2);
        assert_eq!(
            state,
            HostState::Disconnected {
                reason: DisconnectReason::Stopped,
                detail: "the broker stopped when asked".to_owned()
            }
        );
    }

    #[test]
    fn a_new_channel_gets_the_current_state_at_once() {
        let host = Host::new(Trace::new(false));
        host.start(
            &directory("attach"),
            Path::new("/nonexistent/fabric-broker"),
        );
        let first = attached(&host);
        let second = attached(&host);
        assert_eq!(state_of(next(&first)), state_of(next(&second)));
    }

    #[test]
    fn frames_are_described_without_their_content() {
        let line = br#"{"change":{"kind":"snapshot_required"},"protocol":"fabric-desktop/1","sequence":3}"#;
        let frame = decode(Entry::OwnerNotice, line).expect("a valid notice");
        assert_eq!(
            describe_frame(&frame),
            format!(
                "<- notice sequence=3 change=snapshot_required bytes={}",
                line.len()
            )
        );
        assert_eq!(
            describe_request(HELLO),
            format!("-> hello id=hello-1 bytes={}", HELLO.len())
        );
    }
}
