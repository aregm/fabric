//! The host's state: one broker child at most, the connection state, and the webview's channel.
//!
//! Nothing here is Tauri, so it is tested without a window. The host relays and nothing more:
//! it never builds a frame, never answers for the broker and never looks at what a request
//! asks for. It holds no Google credential and no database: it tells the broker where its
//! store lives and where its key comes from ([`BrokerStore`]), and the broker opens it. It also
//! tells it where Isabelle and the rule theory are ([`BrokerProver`]): the installed layout, or
//! in a debug build the developer's three paths; the broker checks them and runs the prover.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use fabric_desktop_contracts::owner::OwnerResponse;
use fabric_desktop_contracts::{Entry, Frame, decode};
use fabric_desktop_relay::{
    BROKER_APP_RESOURCES_DIR as APP_RESOURCES_DIR, BROKER_APP_SUPPORT_DIR as APP_SUPPORT_DIR,
    BROKER_DATA_DIR as DATA_DIR, BROKER_ISABELLE_HOME as ISABELLE_HOME,
    BROKER_ISABELLE_USER_HOME as ISABELLE_USER_HOME, BROKER_KEY_SOURCE as KEY_SOURCE,
    BROKER_RULES_DIR as RULES_DIR, BROKER_SYNTHETIC_AGENT as SYNTHETIC_AGENT, BrokerConfig,
    ENVIRONMENT_ALLOWLIST, Event, InstanceError, InstanceLock, QUIT_DRAIN, Session,
    is_broker_variable, same_name as is_variable,
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

/// Where the broker's store key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerKeySource {
    /// The generic password in the login Keychain: the product.
    Keychain,
    /// The public test key: debug builds started from a terminal with a scratch directory.
    Test,
}

impl BrokerKeySource {
    fn as_str(self) -> &'static str {
        match self {
            BrokerKeySource::Keychain => "keychain",
            BrokerKeySource::Test => "test",
        }
    }
}

/// Where the broker's rule prover finds Isabelle and the rule theory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerProver {
    /// The developer mode: three absolute paths, all or none. Only a debug host passes them,
    /// with any store; the broker checks them before it starts and refuses a wrong one (exit 78).
    Developer {
        /// The Isabelle distribution's root (`FABRIC_BROKER_ISABELLE_HOME`).
        isabelle_home: PathBuf,
        /// Isabelle's per-user directory (`FABRIC_BROKER_ISABELLE_USER_HOME`).
        isabelle_user_home: PathBuf,
        /// The directory with `Fabric_Broker/` (`FABRIC_BROKER_RULES_DIR`).
        rules_dir: PathBuf,
    },
    /// The installed layout: every host that does not pass the developer's paths. The broker
    /// looks for `Isabelle2025-2` in the support folder and runs no prover if it is not there.
    Installed {
        /// The application's resources folder, which holds `isabelle/Fabric_Broker/`
        /// (`FABRIC_BROKER_APP_RESOURCES_DIR`): Tauri's resource folder, `Contents/Resources`
        /// in a bundle.
        resources_dir: PathBuf,
        /// The host's own folder, which holds `instance.lock` and `broker/`
        /// (`FABRIC_BROKER_APP_SUPPORT_DIR`).
        support_dir: PathBuf,
    },
}

/// The store settings the host gives the broker (design 10.6), and where its prover is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerStore {
    /// The broker's data directory: `broker` under the host's directory.
    pub data_dir: PathBuf,
    /// Where the store key comes from.
    pub key_source: BrokerKeySource,
    /// True when the broker runs its synthetic agent (debug builds, test key only).
    pub synthetic_agent: bool,
    /// Where Isabelle and the rule theory are. The host always sets it; `None` (only in tests)
    /// passes none of the prover's variables.
    pub prover: Option<BrokerProver>,
}

/// The variable that moves the host's directory, and with it the lock and the store, in a
/// debug build.
pub const DESKTOP_DATA_DIR: &str = "FABRIC_DESKTOP_DATA_DIR";
/// The broker's directory under the host's.
const BROKER_DIRECTORY: &str = "broker";

impl BrokerStore {
    /// From the host's own environment. Release builds: root = `app_data_dir`, Keychain, no synthetic agent, the
    /// installed layout, all six names ignored. Debug builds: root = `FABRIC_DESKTOP_DATA_DIR` if set; `FABRIC_BROKER_KEY_SOURCE=test` and
    /// `FABRIC_BROKER_SYNTHETIC_AGENT=1` are honoured only with `FABRIC_DESKTOP_DATA_DIR` set, and the synthetic agent
    /// only with the test key; the three prover paths are passed when all three are set and absolute, with any
    /// store, and the installed layout when none is set; otherwise `Err(detail)`. `resources_dir` is the
    /// application's resources folder, `None` when Tauri could not resolve it. Returns the root (the lock
    /// directory) and the store settings (`data_dir = root/broker`).
    pub fn from_environment(
        app_data_dir: &Path,
        resources_dir: Option<&Path>,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> (PathBuf, Result<BrokerStore, String>) {
        let mut desktop = None;
        let mut key = None;
        let mut synthetic = None;
        let mut prover = [None, None, None];
        for (name, value) in vars {
            if is_variable(&name, DESKTOP_DATA_DIR) {
                desktop = Some(value);
            } else if is_variable(&name, KEY_SOURCE) {
                key = Some(value);
            } else if is_variable(&name, SYNTHETIC_AGENT) {
                synthetic = Some(value);
            } else if let Some(index) = PROVER_NAMES.iter().position(|n| is_variable(&name, n)) {
                prover[index] = Some(value);
            }
        }
        let release = cfg!(not(debug_assertions));
        let (root, store) = BrokerStore::store_from(app_data_dir, desktop, key, synthetic, release);
        let prover = BrokerProver::prover_from(prover, release, &root, resources_dir);
        let store = store.and_then(|store| {
            Ok(BrokerStore {
                prover: prover?,
                ..store
            })
        });
        (root, store)
    }

    /// The rule of [`BrokerStore::from_environment`] once the variables are found; `release` is
    /// true in a release build, which ignores all three.
    fn store_from(
        app_data_dir: &Path,
        desktop: Option<OsString>,
        key: Option<OsString>,
        synthetic: Option<OsString>,
        release: bool,
    ) -> (PathBuf, Result<BrokerStore, String>) {
        if release {
            let _ = (desktop, key, synthetic);
            return (
                app_data_dir.to_path_buf(),
                Ok(BrokerStore::under(
                    app_data_dir,
                    BrokerKeySource::Keychain,
                    false,
                )),
            );
        }
        let root = match desktop {
            None => None,
            Some(value) => {
                let root = PathBuf::from(value);
                if !root.is_absolute() {
                    return (
                        app_data_dir.to_path_buf(),
                        Err(format!(
                            "{DESKTOP_DATA_DIR} is {:?}, which is not an absolute path; nothing was started",
                            root.display()
                        )),
                    );
                }
                Some(root)
            }
        };
        let key_source = match key.as_deref().map(OsStr::to_str) {
            None | Some(Some("keychain")) => BrokerKeySource::Keychain,
            Some(Some("test")) => BrokerKeySource::Test,
            Some(_) => {
                return (
                    app_data_dir.to_path_buf(),
                    Err(format!(
                        "{KEY_SOURCE} must be keychain or test; nothing was started"
                    )),
                );
            }
        };
        let synthetic_agent = match synthetic.as_deref().map(OsStr::to_str) {
            None => false,
            Some(Some("1")) => true,
            Some(_) => {
                return (
                    app_data_dir.to_path_buf(),
                    Err(format!(
                        "{SYNTHETIC_AGENT} must be 1 or absent; nothing was started"
                    )),
                );
            }
        };
        let Some(root) = root else {
            if key_source == BrokerKeySource::Test || synthetic_agent {
                return (
                    app_data_dir.to_path_buf(),
                    Err(format!(
                        "the test key and the synthetic agent need {DESKTOP_DATA_DIR}, a scratch directory, so they never touch the real store; nothing was started"
                    )),
                );
            }
            return (
                app_data_dir.to_path_buf(),
                Ok(BrokerStore::under(app_data_dir, key_source, false)),
            );
        };
        if synthetic_agent && key_source != BrokerKeySource::Test {
            return (
                root,
                Err(format!(
                    "the synthetic agent needs {KEY_SOURCE}=test; nothing was started"
                )),
            );
        }
        let store = BrokerStore::under(&root, key_source, synthetic_agent);
        (root, Ok(store))
    }

    fn under(root: &Path, key_source: BrokerKeySource, synthetic_agent: bool) -> BrokerStore {
        BrokerStore {
            data_dir: root.join(BROKER_DIRECTORY),
            key_source,
            synthetic_agent,
            prover: None,
        }
    }

    /// The broker's variables: data dir, key source, the synthetic pair only when `synthetic_agent` is true, then
    /// the three developer paths or the two folders of the installed layout, as `prover` says.
    #[must_use]
    pub fn variables(&self) -> Vec<(OsString, OsString)> {
        let mut variables = vec![
            (
                OsString::from(DATA_DIR),
                self.data_dir.clone().into_os_string(),
            ),
            (
                OsString::from(KEY_SOURCE),
                OsString::from(self.key_source.as_str()),
            ),
        ];
        if self.synthetic_agent {
            variables.push((OsString::from(SYNTHETIC_AGENT), OsString::from("1")));
        }
        match &self.prover {
            None => {}
            Some(BrokerProver::Developer {
                isabelle_home,
                isabelle_user_home,
                rules_dir,
            }) => {
                for (name, path) in
                    PROVER_NAMES
                        .into_iter()
                        .zip([isabelle_home, isabelle_user_home, rules_dir])
                {
                    variables.push((OsString::from(name), path.clone().into_os_string()));
                }
            }
            Some(BrokerProver::Installed {
                resources_dir,
                support_dir,
            }) => {
                for (name, path) in [
                    (APP_RESOURCES_DIR, resources_dir),
                    (APP_SUPPORT_DIR, support_dir),
                ] {
                    variables.push((OsString::from(name), path.clone().into_os_string()));
                }
            }
        }
        variables
    }
}

/// The configuration error of a host that cannot resolve its resources folder: without it the
/// broker would not find the rule theory, so nothing is started.
const NO_RESOURCES: &str = "the application's resources folder, which holds the rule theory, could not be found; nothing was started";

/// The developer's three prover variables, in the order of [`BrokerProver::Developer`]'s fields.
const PROVER_NAMES: [&str; 3] = [ISABELLE_HOME, ISABELLE_USER_HOME, RULES_DIR];

impl BrokerProver {
    /// The rule for the three prover variables once they are found, in the order of
    /// [`PROVER_NAMES`]. A release build ignores them and passes the installed layout. A debug
    /// build passes the developer's paths when all three are set and absolute, and the installed
    /// layout when none is set; one or two set, or a path that is not absolute, is `Err(detail)`.
    /// The installed layout is `resources_dir` and `root`, the host's own folder; it is
    /// `Err(detail)` when `resources_dir` is `None` or not absolute. Whether the paths hold
    /// Isabelle and the rule theory is the broker's check, not the host's.
    fn prover_from(
        values: [Option<OsString>; 3],
        release: bool,
        root: &Path,
        resources_dir: Option<&Path>,
    ) -> Result<Option<BrokerProver>, String> {
        if release || values.iter().all(Option::is_none) {
            return match resources_dir {
                Some(resources_dir) if resources_dir.is_absolute() => {
                    Ok(Some(BrokerProver::Installed {
                        resources_dir: resources_dir.to_path_buf(),
                        support_dir: root.to_path_buf(),
                    }))
                }
                _ => Err(NO_RESOURCES.to_owned()),
            };
        }
        let [Some(home), Some(user_home), Some(rules)] = values else {
            return Err(format!(
                "the rule prover needs {ISABELLE_HOME}, {ISABELLE_USER_HOME} and {RULES_DIR} together, or none; nothing was started"
            ));
        };
        let absolute = |name: &str, value: OsString| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(format!(
                    "{name} is {:?}, which is not an absolute path; nothing was started",
                    path.display()
                ))
            }
        };
        Ok(Some(BrokerProver::Developer {
            isabelle_home: absolute(ISABELLE_HOME, home)?,
            isabelle_user_home: absolute(ISABELLE_USER_HOME, user_home)?,
            rules_dir: absolute(RULES_DIR, rules)?,
        }))
    }
}

/// The host's environment without any `FABRIC_BROKER_*` name, then the store's variables.
fn broker_environment(store: &BrokerStore) -> Vec<(OsString, OsString)> {
    environment_from(std::env::vars_os(), store)
}

/// [`broker_environment`] for the given inherited variables.
fn environment_from(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    store: &BrokerStore,
) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<(OsString, OsString)> = inherited
        .into_iter()
        .filter(|(name, _)| !is_broker_variable(name))
        .collect();
    environment.extend(store.variables());
    environment
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
    /// The store settings, kept for `restart`.
    store: Option<BrokerStore>,
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
                store: None,
            })),
            trace,
        }
    }

    /// Takes the single-instance lock in `lock_directory` and, if this is the only instance,
    /// starts the broker at `broker` with the store settings. A second instance ends in the
    /// `conflict` state and starts nothing. Refused store settings (`Err(detail)`) end in the
    /// `configuration_error` state: nothing is locked or started.
    pub fn start(&self, lock_directory: &Path, broker: &Path, store: Result<BrokerStore, String>) {
        let mut inner = self.lock();
        inner.broker = broker.to_path_buf();
        match store {
            Ok(store) => inner.store = Some(store),
            Err(detail) => {
                inner.set_state(HostState::ConfigurationError { detail }, self.trace, "");
                return;
            }
        }
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
    /// starts nothing (lesson DEF-2 of the demo). The broker's environment is the host's
    /// without any inherited `FABRIC_BROKER_*` name, then the store's variables, filtered by
    /// the allowlist.
    fn start_broker(&self, inner: &mut Inner) {
        let Some(store) = inner.store.clone() else {
            let detail = "the broker's store settings are missing".to_owned();
            inner.set_state(HostState::ConfigurationError { detail }, self.trace, "");
            return;
        };
        let environment = broker_environment(&store);
        let config = match BrokerConfig::check(&inner.broker, ENVIRONMENT_ALLOWLIST, environment) {
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
                    " pid={} broker={:?} key={}{}{}",
                    session.pid(),
                    config.program().display().to_string(),
                    store.key_source.as_str(),
                    if store.synthetic_agent {
                        " synthetic"
                    } else {
                        ""
                    },
                    match store.prover {
                        Some(BrokerProver::Developer { .. }) => " prover",
                        Some(BrokerProver::Installed { .. }) => " installed",
                        None => "",
                    }
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

    const HELLO: &[u8] = br#"{"protocol":"fabric-desktop/1","id":"hello-1","method":"hello","params":{"client":{"name":"host test","build":"0"},"schema_version":2}}"#;
    const SNAPSHOT: &[u8] =
        br#"{"protocol":"fabric-desktop/1","id":"snap-1","method":"get_snapshot","params":{}}"#;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "fabric-desktop-host-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    /// The store of a test: `broker` under the test's directory, with the public test key.
    /// No host test gives the broker the Keychain.
    fn test_store(directory: &Path) -> Result<BrokerStore, String> {
        Ok(BrokerStore {
            data_dir: directory.join("broker"),
            key_source: BrokerKeySource::Test,
            synthetic_agent: false,
            prover: None,
        })
    }

    /// The resources folder the tests hand the host, as Tauri's `resource_dir()` would. Nothing
    /// is created: the host does not look at it.
    const RESOURCES: &str = "/Applications/Fabric Calendar.app/Contents/Resources";

    /// The installed layout of a host whose own folder is `support`.
    fn installed(support: &Path) -> Option<BrokerProver> {
        Some(BrokerProver::Installed {
            resources_dir: PathBuf::from(RESOURCES),
            support_dir: support.to_path_buf(),
        })
    }

    fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
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
        host.start(
            &lock_directory,
            Path::new("/nonexistent/fabric-broker"),
            test_store(&lock_directory),
        );
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
        host.start(
            &lock_directory,
            Path::new(STAGED_BROKER),
            test_store(&lock_directory),
        );
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
        let lock_directory = directory("relative");
        host.start(
            &lock_directory,
            Path::new("fabric-broker"),
            test_store(&lock_directory),
        );
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
        let lock_directory = directory("round-trip");
        let store = test_store(&lock_directory);
        let data_dir = store.clone().unwrap().data_dir;
        host.start(&lock_directory, Path::new(STAGED_BROKER), store);
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
                assert_eq!(frame["result"]["schema_version"], 2);
            }
            other => panic!("expected the hello answer, got {other:?}"),
        }
        // The broker opened its store in the test's directory, not anywhere else.
        host.send(SNAPSHOT).expect("get_snapshot is a valid frame");
        match next(&receiver) {
            HostMessage::Frame { run, frame } => {
                assert_eq!(run, 1);
                assert_eq!(frame["result"]["health"]["store"], "ready");
            }
            other => panic!("expected the snapshot, got {other:?}"),
        }
        assert!(data_dir.join("broker.db").is_file());

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

        // The owner restarts: a new run on the same store, and the round trip works again.
        host.restart()
            .expect("a disconnected broker can be restarted");
        assert_eq!(state_of(next(&receiver)), (2, HostState::Connected));
        host.send(HELLO).expect("hello is a valid frame");
        assert!(matches!(next(&receiver), HostMessage::Frame { run: 2, .. }));
        host.send(SNAPSHOT).expect("get_snapshot is a valid frame");
        match next(&receiver) {
            HostMessage::Frame { run, frame } => {
                assert_eq!(run, 2);
                assert_eq!(frame["result"]["health"]["store"], "ready");
            }
            other => panic!("expected the snapshot, got {other:?}"),
        }

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
    fn a_refused_store_setting_is_a_configuration_error_and_nothing_starts() {
        let host = Host::new(Trace::new(false));
        let lock_directory = directory("refused-store");
        host.start(
            &lock_directory,
            Path::new(STAGED_BROKER),
            Err("the test key needs a scratch directory".to_owned()),
        );
        let receiver = attached(&host);
        let (run, state) = state_of(next(&receiver));
        assert_eq!(run, 0, "no broker start was counted");
        assert_eq!(
            state,
            HostState::ConfigurationError {
                detail: "the test key needs a scratch directory".to_owned()
            }
        );
        assert!(!lock_directory.exists(), "nothing was locked");
        assert_eq!(host.send(HELLO), Err(SendRefusal::NotConnected));
        assert!(host.restart().is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn a_debug_host_takes_the_test_key_only_with_a_scratch_directory() {
        let app = Path::new("/Users/someone/Library/Application Support/app");
        let scratch = std::env::temp_dir().join("fabric-scratch");
        let scratch_text = scratch.to_string_lossy().into_owned();

        let resources = Some(Path::new(RESOURCES));

        // Nothing set: the product's store under the application's directory, the Keychain, and
        // the installed layout in the application's directory.
        let (root, store) = BrokerStore::from_environment(app, resources, vars(&[("HOME", "/h")]));
        assert_eq!(root, app);
        assert_eq!(
            store,
            Ok(BrokerStore {
                data_dir: app.join("broker"),
                key_source: BrokerKeySource::Keychain,
                synthetic_agent: false,
                prover: installed(app),
            })
        );

        // The synthetic run of the README: the scratch directory is also where the broker
        // looks for Isabelle, so no test touches the real folder.
        let (root, store) = BrokerStore::from_environment(
            app,
            resources,
            vars(&[
                (DESKTOP_DATA_DIR, &scratch_text),
                (KEY_SOURCE, "test"),
                (SYNTHETIC_AGENT, "1"),
            ]),
        );
        assert_eq!(root, scratch);
        let store = store.unwrap();
        assert_eq!(store.data_dir, scratch.join("broker"));
        assert_eq!(store.key_source, BrokerKeySource::Test);
        assert!(store.synthetic_agent);
        assert_eq!(store.prover, installed(&scratch));
        assert_eq!(
            store.variables(),
            vec![
                (
                    OsString::from(DATA_DIR),
                    scratch.join("broker").into_os_string()
                ),
                (OsString::from(KEY_SOURCE), OsString::from("test")),
                (OsString::from(SYNTHETIC_AGENT), OsString::from("1")),
                (OsString::from(APP_RESOURCES_DIR), OsString::from(RESOURCES)),
                (
                    OsString::from(APP_SUPPORT_DIR),
                    scratch.clone().into_os_string()
                ),
            ]
        );

        // The two folders in the host's own environment are not read: the host computes them.
        let (_, store) = BrokerStore::from_environment(
            app,
            resources,
            vars(&[
                (DESKTOP_DATA_DIR, &scratch_text),
                (APP_RESOURCES_DIR, "/inherited/resources"),
                (APP_SUPPORT_DIR, "/inherited/support"),
            ]),
        );
        assert_eq!(store.unwrap().prover, installed(&scratch));

        // Refused: the test key or the synthetic agent on the real store, the synthetic agent
        // with the Keychain, unknown values, a relative scratch directory.
        for refused in [
            vars(&[(KEY_SOURCE, "test")]),
            vars(&[(SYNTHETIC_AGENT, "1")]),
            vars(&[(KEY_SOURCE, "test"), (SYNTHETIC_AGENT, "1")]),
            vars(&[(DESKTOP_DATA_DIR, &scratch_text), (SYNTHETIC_AGENT, "1")]),
            vars(&[(DESKTOP_DATA_DIR, &scratch_text), (KEY_SOURCE, "file")]),
            vars(&[(DESKTOP_DATA_DIR, &scratch_text), (SYNTHETIC_AGENT, "yes")]),
            vars(&[(DESKTOP_DATA_DIR, "relative"), (KEY_SOURCE, "test")]),
        ] {
            let (_, store) = BrokerStore::from_environment(app, resources, refused.clone());
            assert!(store.is_err(), "{refused:?} was accepted: {store:?}");
        }

        // A scratch directory alone moves the store and keeps the Keychain.
        let (root, store) = BrokerStore::from_environment(
            app,
            resources,
            vars(&[(DESKTOP_DATA_DIR, &scratch_text)]),
        );
        assert_eq!(root, scratch);
        assert_eq!(store.unwrap().key_source, BrokerKeySource::Keychain);
    }

    #[test]
    fn a_release_host_ignores_all_three() {
        let app = Path::new("/Users/someone/Library/Application Support/app");
        let product = Ok(BrokerStore {
            data_dir: app.join("broker"),
            key_source: BrokerKeySource::Keychain,
            synthetic_agent: false,
            prover: None,
        });
        let scratch = std::env::temp_dir().join("fabric-scratch").into_os_string();
        let some = |text: &str| Some(OsString::from(text));
        for (desktop, key, synthetic) in [
            // The synthetic run of the README, which a debug build honours.
            (Some(scratch.clone()), some("test"), some("1")),
            // Values a debug build refuses.
            (Some(scratch.clone()), some("file"), some("yes")),
            (some("relative"), some("test"), None),
            (None, some("test"), some("1")),
            (None, None, None),
        ] {
            let case = format!("{desktop:?} {key:?} {synthetic:?}");
            let (root, store) = BrokerStore::store_from(app, desktop, key, synthetic, true);
            assert_eq!(root, app, "{case}");
            assert_eq!(store, product, "{case}");
        }
    }

    /// Three absolute prover paths under the temporary directory, as text, in the order of
    /// [`PROVER_NAMES`]. Nothing is created: the host does not look at them.
    fn prover_paths() -> [String; 3] {
        let base = std::env::temp_dir().join("fabric-prover-paths");
        ["Isabelle2025-2", "home", "isabelle"]
            .map(|leaf| base.join(leaf).to_string_lossy().into_owned())
    }

    #[cfg(debug_assertions)]
    #[test]
    fn a_debug_host_passes_the_prover_paths_only_all_three_and_absolute() {
        let app = Path::new("/Users/someone/Library/Application Support/app");
        let scratch = std::env::temp_dir().join("fabric-scratch");
        let scratch_text = scratch.to_string_lossy().into_owned();
        let [home, user_home, rules] = prover_paths();
        let expected = BrokerProver::Developer {
            isabelle_home: PathBuf::from(&home),
            isabelle_user_home: PathBuf::from(&user_home),
            rules_dir: PathBuf::from(&rules),
        };
        let resources = Some(Path::new(RESOURCES));

        // The practice run of the README: the synthetic run and the three paths. The developer
        // mode passes neither folder of the installed layout.
        let (root, store) = BrokerStore::from_environment(
            app,
            resources,
            vars(&[
                (DESKTOP_DATA_DIR, &scratch_text),
                (KEY_SOURCE, "test"),
                (SYNTHETIC_AGENT, "1"),
                (ISABELLE_HOME, &home),
                (ISABELLE_USER_HOME, &user_home),
                (RULES_DIR, &rules),
            ]),
        );
        assert_eq!(root, scratch);
        let store = store.unwrap();
        assert_eq!(store.prover.as_ref(), Some(&expected));
        assert_eq!(
            store.variables(),
            vec![
                (
                    OsString::from(DATA_DIR),
                    scratch.join("broker").into_os_string()
                ),
                (OsString::from(KEY_SOURCE), OsString::from("test")),
                (OsString::from(SYNTHETIC_AGENT), OsString::from("1")),
                (OsString::from(ISABELLE_HOME), OsString::from(&home)),
                (
                    OsString::from(ISABELLE_USER_HOME),
                    OsString::from(&user_home)
                ),
                (OsString::from(RULES_DIR), OsString::from(&rules)),
            ]
        );

        // With any store: the product's store and the Keychain keep the paths too. The developer
        // mode needs no resources folder.
        let (root, store) = BrokerStore::from_environment(
            app,
            None,
            vars(&[
                (ISABELLE_HOME, &home),
                (ISABELLE_USER_HOME, &user_home),
                (RULES_DIR, &rules),
            ]),
        );
        assert_eq!(root, app);
        let store = store.unwrap();
        assert_eq!(store.key_source, BrokerKeySource::Keychain);
        assert!(!store.synthetic_agent);
        assert_eq!(store.prover, Some(expected));

        // None of the three: the installed layout in the host's folder, and none of the three.
        let (_, store) = BrokerStore::from_environment(
            app,
            resources,
            vars(&[(DESKTOP_DATA_DIR, &scratch_text), (KEY_SOURCE, "test")]),
        );
        let store = store.unwrap();
        assert_eq!(store.prover, installed(&scratch));
        assert!(
            store
                .variables()
                .iter()
                .all(|(name, _)| !PROVER_NAMES.iter().any(|n| is_variable(name, n)))
        );

        // Refused: one or two of the three, a relative or empty path, and a refused store that
        // has all three.
        for refused in [
            vars(&[(ISABELLE_HOME, &home)]),
            vars(&[(ISABELLE_USER_HOME, &user_home), (RULES_DIR, &rules)]),
            vars(&[(ISABELLE_HOME, &home), (RULES_DIR, &rules)]),
            vars(&[
                (ISABELLE_HOME, ".tooling/isabelle/Isabelle2025-2"),
                (ISABELLE_USER_HOME, &user_home),
                (RULES_DIR, &rules),
            ]),
            vars(&[
                (ISABELLE_HOME, &home),
                (ISABELLE_USER_HOME, &user_home),
                (RULES_DIR, ""),
            ]),
            vars(&[
                (KEY_SOURCE, "test"),
                (ISABELLE_HOME, &home),
                (ISABELLE_USER_HOME, &user_home),
                (RULES_DIR, &rules),
            ]),
        ] {
            let (_, store) = BrokerStore::from_environment(app, resources, refused.clone());
            assert!(store.is_err(), "{refused:?} was accepted: {store:?}");
        }
        let (_, partial) =
            BrokerStore::from_environment(app, resources, vars(&[(RULES_DIR, &rules)]));
        assert_eq!(
            partial,
            Err(
                "the rule prover needs FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR together, or none; nothing was started"
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_release_host_ignores_the_prover_variables() {
        let app = Path::new("/Users/someone/Library/Application Support/app");
        let resources = Some(Path::new(RESOURCES));
        let [home, user_home, rules] = prover_paths();
        let some = |text: &str| Some(OsString::from(text));
        for values in [
            // All three, which a debug build passes.
            [some(&home), some(&user_home), some(&rules)],
            // Values a debug build refuses.
            [some(&home), None, None],
            [some("relative"), some(&user_home), some(&rules)],
            [None, None, None],
        ] {
            let case = format!("{values:?}");
            // The installed layout, never the developer's paths.
            assert_eq!(
                BrokerProver::prover_from(values.clone(), true, app, resources),
                Ok(installed(app)),
                "{case}"
            );
            // Without its resources folder a release host starts nothing, whatever is set.
            assert_eq!(
                BrokerProver::prover_from(values, true, app, None),
                Err(NO_RESOURCES.to_owned()),
                "{case}"
            );
        }
    }

    #[test]
    fn a_host_without_its_resources_folder_passes_no_installed_layout() {
        let app = Path::new("/Users/someone/Library/Application Support/app");
        let [home, user_home, rules] = prover_paths();
        let some = |text: &str| Some(OsString::from(text));
        // A debug host with none of the three needs the resources folder: not resolved, or not
        // an absolute path, is the configuration error.
        for resources in [None, Some(Path::new("Contents/Resources"))] {
            assert_eq!(
                BrokerProver::prover_from([None, None, None], false, app, resources),
                Err(NO_RESOURCES.to_owned()),
                "{resources:?}"
            );
        }
        // The developer mode does not use it.
        assert_eq!(
            BrokerProver::prover_from(
                [some(&home), some(&user_home), some(&rules)],
                false,
                app,
                None
            ),
            Ok(Some(BrokerProver::Developer {
                isabelle_home: PathBuf::from(&home),
                isabelle_user_home: PathBuf::from(&user_home),
                rules_dir: PathBuf::from(&rules),
            }))
        );
        // The host turns the error into its configuration error and starts nothing.
        let host = Host::new(Trace::new(false));
        let lock_directory = directory("no-resources");
        host.start(
            &lock_directory,
            Path::new(STAGED_BROKER),
            Err(NO_RESOURCES.to_owned()),
        );
        let (run, state) = state_of(next(&attached(&host)));
        assert_eq!(run, 0);
        assert_eq!(
            state,
            HostState::ConfigurationError {
                detail: NO_RESOURCES.to_owned()
            }
        );
        assert!(!lock_directory.exists(), "nothing was locked");
    }

    #[test]
    fn every_broker_variable_the_host_sets_passes_the_relay_allowlist() {
        let mut store = BrokerStore {
            data_dir: PathBuf::from("/scratch/broker"),
            key_source: BrokerKeySource::Test,
            synthetic_agent: true,
            prover: None,
        };
        assert_eq!(store.variables().len(), 3);
        store.prover = Some(BrokerProver::Developer {
            isabelle_home: PathBuf::from("/isabelle/Isabelle2025-2"),
            isabelle_user_home: PathBuf::from("/isabelle/home"),
            rules_dir: PathBuf::from("/rules"),
        });
        let developer = store.variables();
        assert_eq!(developer.len(), 6);
        store.prover = installed(Path::new("/support"));
        let installed = store.variables();
        assert_eq!(installed.len(), 5);
        for (name, _) in developer.iter().chain(&installed) {
            assert!(is_broker_variable(name), "{name:?}");
            assert!(
                ENVIRONMENT_ALLOWLIST
                    .iter()
                    .any(|allowed| is_variable(name, allowed)),
                "{name:?} would not reach the broker"
            );
        }
    }

    #[test]
    fn the_broker_environment_drops_inherited_broker_variables() {
        let store = BrokerStore {
            data_dir: PathBuf::from("/scratch/broker"),
            key_source: BrokerKeySource::Test,
            synthetic_agent: false,
            prover: None,
        };
        let environment = broker_environment(&store);
        let broker: Vec<String> = environment
            .iter()
            .map(|(name, _)| name)
            .filter(|name| is_broker_variable(name))
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(broker, [DATA_DIR, KEY_SOURCE]);
        // The installed layout's two folders are the host's own values, never inherited ones:
        // each name appears once, with the value the host computed.
        let store = BrokerStore {
            prover: installed(Path::new("/scratch")),
            ..store
        };
        let inherited = vars(&[
            ("HOME", "/h"),
            (APP_RESOURCES_DIR, "/inherited/resources"),
            (APP_SUPPORT_DIR, "/inherited/support"),
            (DATA_DIR, "/inherited/broker"),
        ]);
        let environment = environment_from(inherited, &store);
        assert!(environment.contains(&(OsString::from("HOME"), OsString::from("/h"))));
        let broker: Vec<(String, String)> = environment
            .iter()
            .filter(|(name, _)| is_broker_variable(name))
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(
            broker,
            [
                (DATA_DIR, "/scratch/broker"),
                (KEY_SOURCE, "test"),
                (APP_RESOURCES_DIR, RESOURCES),
                (APP_SUPPORT_DIR, "/scratch"),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
        );
        assert!(is_broker_variable(OsStr::new("FABRIC_BROKER_UNDER_TEST")));
        assert!(!is_broker_variable(OsStr::new("FABRIC_DESKTOP_DATA_DIR")));
    }

    #[test]
    fn a_new_channel_gets_the_current_state_at_once() {
        let host = Host::new(Trace::new(false));
        let lock_directory = directory("attach");
        host.start(
            &lock_directory,
            Path::new("/nonexistent/fabric-broker"),
            test_store(&lock_directory),
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
