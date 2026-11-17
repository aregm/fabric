//! The broker's proof gate: it renders the trusted context and one action into a check
//! theory, has a running Isabelle server prove every rule or its negation, and returns the
//! verdict that follows with its proof id (plan section 8). It is modelled on
//! [`fabric_policy::Gate`] and uses the same server and connection.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use fabric_policy::PolicyError;
use fabric_policy::server::{
    Connection, KillHandle, Server, ServerAddress, ServerOptions, platform_path,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::check::{self, CheckTheory, GrantResult, NodeReport, RuleResult, Verdict};
use crate::model::{Action, Context, Rule};
use crate::report::{context_digest, proof_id, sha256_hex};
use crate::theory::SESSION_ROOT;

/// The Isabelle session of the rule theory (G0-THY).
pub const SESSION: &str = "Fabric_Broker";
const SESSION_START_TIMEOUT: Duration = Duration::from_secs(600);

/// How long after a check's limit the gate still waits for Isabelle's answers to
/// `use_theories` and `purge_theories`. Isabelle ends the check at the limit itself and
/// answers within a fraction of a second; a server that has not answered by then is taken
/// to be stuck, and the gate is broken.
pub const CHECK_GRACE: Duration = Duration::from_secs(5);

/// How often Isabelle looks at a running check (`check_delay` of `use_theories`). The check's
/// limit is given to Isabelle as a number of these looks.
const CHECK_DELAY: Duration = Duration::from_millis(50);

/// The bound of [`BrokerGate::stop`].
pub const STOP_BOUND: Duration = Duration::from_secs(20);

/// The bound of the stop a gate does when it is dropped without [`BrokerGate::stop`]: shorter,
/// because a drop may happen on the way out of a failing caller.
pub const DROP_STOP_BOUND: Duration = Duration::from_secs(5);

/// How often [`BrokerGate::stop_within`] looks whether the server has exited.
const EXIT_POLL: Duration = Duration::from_millis(10);

/// How long past its bound [`BrokerGate::stop_within`] may still spend delivering `shutdown`
/// to a server that `isabelle server` found already running, when the bounded exchange did not
/// get it acknowledged: connecting, the greeting and the write take a few milliseconds.
pub const FOUND_SHUTDOWN_GRACE: Duration = Duration::from_millis(300);

/// The settings line that keeps Isabelle's temporary files in its user home.
#[cfg(unix)]
pub const TMP_PREFIX_SETTING: &str = "ISABELLE_TMP_PREFIX=\"$ISABELLE_HOME_USER/tmp\"";

#[derive(Clone, Debug)]
pub struct GateConfig {
    pub isabelle_home: PathBuf,
    pub user_home: PathBuf,
    /// The directory with `Fabric_Broker/`, which holds the session's `ROOT` and
    /// `Fabric_Broker.thy`. The session is loaded from `Fabric_Broker/` alone, so no `ROOTS`
    /// file is needed, and the gate refuses one there: Isabelle reads `ROOT` and `ROOTS` of
    /// that directory, so `ROOT` must be the one the crate was built with
    /// ([`crate::theory::SESSION_ROOT`]) and a `ROOTS` must not exist
    /// ([`check_session_dir`]). A `ROOTS` in `rules_dir` itself, as in the repository's
    /// `isabelle/`, is not read.
    pub rules_dir: PathBuf,
    /// Where check theories and server logs are written.
    pub work_dir: PathBuf,
    pub server_name: String,
    /// The limit for one check, counted from its start (plan section 5: 30 seconds). It is
    /// Isabelle's watchdog, and a check that takes longer is undecided whatever it proved.
    pub check_timeout: Duration,
    /// `headless_consolidate_delay` for the session, in seconds.
    pub consolidate_delay_seconds: f64,
}

/// The Isabelle distribution the rule theory and its checks were verified with. A
/// distribution names itself in `etc/ISABELLE_IDENTIFIER`, and its server reports the same
/// name.
pub const ISABELLE_VERSION: &str = "Isabelle2025-2";

/// In an installation's support folder, the folder that holds the Isabelle distribution: its
/// `bin/isabelle` is directly inside.
pub const INSTALLED_ISABELLE_DIR: &str = ISABELLE_VERSION;

/// In an installation's support folder, Isabelle's user home. Isabelle keeps the heap of the
/// `Fabric_Broker` session, its server registry and its temporary files below it.
pub const INSTALLED_USER_HOME_DIR: &str = "isabelle-user-home";

/// In an application bundle's resources, the folder that holds `Fabric_Broker/`.
pub const INSTALLED_RULES_DIR: &str = "isabelle";

impl GateConfig {
    /// Defaults for a repository checkout set up with `scripts/setup-demo.sh`.
    #[must_use]
    pub fn for_repository(root: &Path) -> Self {
        let isabelle = root.join(".tooling").join("isabelle");
        Self {
            isabelle_home: isabelle.join(ISABELLE_VERSION),
            user_home: isabelle.join("home"),
            rules_dir: root.join("isabelle"),
            work_dir: root.join(".local").join("broker-checks"),
            server_name: "fabric-broker".to_owned(),
            check_timeout: Duration::from_secs(30),
            consolidate_delay_seconds: 0.1,
        }
    }

    /// For an installed application (plan Q09): the rule theory comes with the application,
    /// and Isabelle is installed once into a fixed folder of the user.
    ///
    /// - `resources_dir` is the application bundle's resources. The theory is read from
    ///   `<resources_dir>/isabelle/Fabric_Broker/`. Nothing is written there, so the bundle
    ///   may be read-only and signed.
    /// - `support_dir` is the application's own folder for this user. The distribution is
    ///   `<support_dir>/Isabelle2025-2`, and Isabelle's user home is
    ///   `<support_dir>/isabelle-user-home`, which the gate creates.
    /// - `work_dir` and `server_name` are the caller's, as for any other configuration.
    ///
    /// The paths may contain spaces. [`isabelle_installation`] says whether the distribution
    /// is there. The first start builds the session's heap in the user home, and so does the
    /// first start after the application was moved or its theory changed.
    #[must_use]
    pub fn for_installation(
        resources_dir: &Path,
        support_dir: &Path,
        work_dir: &Path,
        server_name: &str,
    ) -> Self {
        Self {
            isabelle_home: support_dir.join(INSTALLED_ISABELLE_DIR),
            user_home: support_dir.join(INSTALLED_USER_HOME_DIR),
            rules_dir: resources_dir.join(INSTALLED_RULES_DIR),
            work_dir: work_dir.to_path_buf(),
            server_name: server_name.to_owned(),
            check_timeout: Duration::from_secs(30),
            consolidate_delay_seconds: 0.1,
        }
    }
}

/// What an Isabelle home holds, as [`isabelle_installation`] reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IsabelleInstallation {
    /// No Isabelle: `bin/isabelle` is not a file there.
    Missing,
    /// An Isabelle that is not [`ISABELLE_VERSION`]. It holds the name the distribution
    /// gives itself, which is empty when it gives none.
    OtherVersion(String),
    /// The distribution the rule theory was verified with.
    Found,
}

/// Reads what `isabelle_home` holds from two of its files, `bin/isabelle` and
/// `etc/ISABELLE_IDENTIFIER`. Nothing is run, so a caller can say that Isabelle is not
/// installed before it starts a gate. It does not tell whether that Isabelle works.
pub fn isabelle_installation(isabelle_home: &Path) -> io::Result<IsabelleInstallation> {
    if !isabelle_home.join("bin").join("isabelle").is_file() {
        return Ok(IsabelleInstallation::Missing);
    }
    let identifier = match fs::read_to_string(isabelle_home.join("etc").join("ISABELLE_IDENTIFIER"))
    {
        Ok(text) => text.trim().to_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    if identifier == ISABELLE_VERSION {
        Ok(IsabelleInstallation::Found)
    } else {
        Ok(IsabelleInstallation::OtherVersion(identifier))
    }
}

/// Everything about one check: for the proof report, the ruling and the audit record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// The check theory's name, such as `Check_000001`.
    pub theory: String,
    pub theory_text: String,
    /// SHA-256 over [`crate::report::PROOF_DOMAIN`], the theory hash, the Isabelle version and
    /// the check theory text: 64 hex characters.
    pub proof_id: String,
    /// SHA-256 of `Fabric_Broker.thy` as loaded: 64 hex characters.
    pub theory_sha256: String,
    pub isabelle: String,
    /// `sha256:` and 64 hex characters ([`crate::report::context_digest`]).
    pub context_digest: String,
    /// The action digest the context names (`ctx_digest`): 64 hex characters.
    pub action_digest: String,
    pub verdict: Verdict,
    pub failing_rule: Option<Rule>,
    pub rules: Vec<RuleResult>,
    /// Every grant of the context in order, with what Isabelle proved about it.
    pub grants: Vec<GrantResult>,
    /// The ids of the grants proven to authorize the action, in context order; empty unless
    /// decided. The broker names them in `ctx_relied` of the request's later checks.
    pub authorizing_grants: Vec<String>,
    pub seconds: f64,
}

/// Whose fault an error of the gate is, and so what a caller does next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    /// The request could not be written for Isabelle; the gate is untouched and can check
    /// the next request.
    Request,
    /// The gate cannot be relied on any more: stop it and start another.
    Gate,
}

impl Fault {
    /// [`PolicyError::Render`] is the request's fault; every other error is the gate's.
    #[must_use]
    pub fn of(error: &PolicyError) -> Fault {
        match error {
            PolicyError::Render(_) => Fault::Request,
            PolicyError::Protocol(_)
            | PolicyError::Isabelle(_)
            | PolicyError::Io(_)
            | PolicyError::Json(_) => Fault::Gate,
        }
    }
}

/// A running Isabelle session with the rule theory loaded.
pub struct BrokerGate {
    config: GateConfig,
    /// The server, when this gate started it.
    server: Option<Server>,
    /// The handle that kills the server from any thread; for a connected gate it reaches
    /// nothing.
    kill: KillHandle,
    /// The `isabelle server` process this gate started had already exited when the session
    /// was open: it found a server already running under the name, which no kill reaches.
    found: bool,
    /// Where the server listens, for a fresh connection when this one is broken.
    address: ServerAddress,
    connection: Connection,
    session_id: String,
    master_dir: String,
    theory_sha256: String,
    isabelle: String,
    next_check: u64,
    stopped: bool,
    /// An exchange with the server, or the work directory, failed: the connection may hold
    /// an answer nobody read, so no further check is sent on it.
    broken: bool,
}

impl BrokerGate {
    /// Starts the Isabelle server and opens a `Fabric_Broker` session, building its heap if
    /// needed. Stopping the gate stops the server.
    ///
    /// On Unix the server runs in a process group of its own, so a stop that runs out of time
    /// can end all of it, and Isabelle's temporary files go to a directory of the user home
    /// only its owner can enter ([`prepare_private_tmp`]). It is
    /// [`BrokerGate::start_with_kill`] with a new handle.
    pub fn start(config: GateConfig) -> Result<Self, PolicyError> {
        Self::start_with_kill(config, &KillHandle::new())
    }

    /// [`BrokerGate::start`], killable through `kill` from any thread while it starts and
    /// afterwards ([`BrokerGate::kill_handle`] returns the same handle). A kill during the
    /// start ends the server and makes the start an error soon after; a handle killed before
    /// the start refuses it. A kill during a check makes it fail ([`Fault::Gate`]) and breaks
    /// the gate, and the following stop returns at once. See [`KillHandle`] for what it cannot
    /// reach.
    pub fn start_with_kill(config: GateConfig, kill: &KillHandle) -> Result<Self, PolicyError> {
        let valid_name = !config.server_name.is_empty()
            && config
                .server_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid_name {
            return Err(PolicyError::Render(format!(
                "server name {:?} must use only letters, digits, '-' and '_'",
                config.server_name
            )));
        }
        prepare_work_dir(&config.work_dir)?;
        #[cfg(unix)]
        require_private_tmp(&config)?;
        let mut server = Server::start_in_group_with(
            &ServerOptions {
                isabelle_home: &config.isabelle_home,
                user_home: &config.user_home,
                name: &config.server_name,
                log_dir: &config.work_dir,
            },
            kill,
        )?;
        match Connection::open(server.port, &server.password, SESSION_START_TIMEOUT) {
            Ok((connection, greeting)) => {
                let address = server.address();
                Self::open(
                    config,
                    Some(server),
                    kill.clone(),
                    address,
                    connection,
                    &greeting,
                )
            }
            Err(error) => {
                server.kill();
                Err(error)
            }
        }
    }

    /// Connects to a server someone else started and opens a session on it. Stopping the
    /// gate stops only that session.
    pub fn connect(config: GateConfig, address: &ServerAddress) -> Result<Self, PolicyError> {
        prepare_work_dir(&config.work_dir)?;
        let (connection, greeting) =
            Connection::open(address.port, &address.password, SESSION_START_TIMEOUT)?;
        Self::open(
            config,
            None,
            KillHandle::new(),
            address.clone(),
            connection,
            &greeting,
        )
    }

    fn open(
        config: GateConfig,
        server: Option<Server>,
        kill: KillHandle,
        address: ServerAddress,
        connection: Connection,
        greeting: &Value,
    ) -> Result<Self, PolicyError> {
        let isabelle = greeting["isabelle_name"]
            .as_str()
            .map_or_else(|| greeting.to_string(), str::to_owned);
        let master_dir = platform_path(&config.work_dir);
        let mut gate = Self {
            config,
            server,
            kill,
            found: false,
            address,
            connection,
            session_id: String::new(),
            master_dir,
            theory_sha256: String::new(),
            isabelle,
            next_check: 1,
            stopped: false,
            broken: false,
        };
        if let Err(error) = gate.open_session() {
            let _ = gate.stop();
            return Err(error);
        }
        // Starting the session took seconds; the wrapper of a server that was found exits
        // within a fraction of one.
        gate.found = gate.server.as_mut().is_some_and(Server::has_exited);
        Ok(gate)
    }

    /// Opens the session from `<rules_dir>/Fabric_Broker` and records the hash of the theory
    /// it loaded. The directory is checked ([`check_session_dir`]) before `session_start` and
    /// again after it, and the two hashes must agree, so the hash names the text Isabelle read.
    fn open_session(&mut self) -> Result<(), PolicyError> {
        let session_dir = self.config.rules_dir.join(SESSION);
        let theory_path = session_dir.join("Fabric_Broker.thy");
        let before = check_session_dir(&session_dir)?;
        let session = self.connection.task(
            "session_start",
            &json!({
                "session": SESSION,
                "dirs": [platform_path(&session_dir)],
                "options": [format!(
                    "headless_consolidate_delay={}",
                    self.config.consolidate_delay_seconds
                )]
            }),
        )?;
        self.session_id = session["session_id"]
            .as_str()
            .ok_or_else(|| PolicyError::Protocol(format!("session_start returned {session}")))?
            .to_owned();
        let after = check_session_dir(&session_dir)?;
        if after != before {
            return Err(PolicyError::Isabelle(format!(
                "{} changed while the session started (SHA-256 {before} before, {after} after)",
                theory_path.display()
            )));
        }
        self.theory_sha256 = before;
        self.connection
            .set_timeout(self.config.check_timeout + Duration::from_secs(30))?;
        Ok(())
    }

    /// The SHA-256 of `Fabric_Broker.thy` as loaded, in hex.
    #[must_use]
    pub fn theory_sha256(&self) -> &str {
        &self.theory_sha256
    }

    /// The Isabelle version the server reported.
    #[must_use]
    pub fn isabelle(&self) -> &str {
        &self.isabelle
    }

    /// The handle that kills this gate's server from any thread ([`KillHandle`]): the one
    /// given to [`BrokerGate::start_with_kill`]. For a gate made with [`BrokerGate::connect`]
    /// it reaches nothing.
    #[must_use]
    pub fn kill_handle(&self) -> KillHandle {
        self.kill.clone()
    }

    /// The port of the server the gate talks to.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.address.port
    }

    /// Whether an earlier failure broke the gate: its checks then fail at once with
    /// [`PolicyError::Protocol`] ([`Fault::Gate`]), and it should be stopped and replaced.
    #[must_use]
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Checks one action in one context within `config.check_timeout`
    /// ([`BrokerGate::check_within`]).
    pub fn check(&mut self, context: &Context, action: &Action) -> Result<Check, PolicyError> {
        self.check_within(context, action, self.config.check_timeout)
    }

    /// Checks one action in one context within `limit`, or `config.check_timeout` if that is
    /// smaller. A context or action that cannot be rendered is an error
    /// ([`Fault::Request`]); a check Isabelle did not finish, or finished out of time, is
    /// undecided. Isabelle is given the limit twice: as the number of times it looks at the
    /// running check (`check_limit`), which bounds the whole check, and as its watchdog,
    /// which ends a check that has made no progress for that long. Either way Isabelle
    /// itself ends the check and the gate stays usable. A server that has not answered by
    /// the limit and [`CHECK_GRACE`], or any other failure of the connection or the work
    /// directory, breaks the gate ([`BrokerGate::is_broken`]).
    pub fn check_within(
        &mut self,
        context: &Context,
        action: &Action,
        limit: Duration,
    ) -> Result<Check, PolicyError> {
        if self.broken {
            return Err(PolicyError::Protocol(
                "the gate is broken by an earlier failure; stop it and start another".to_owned(),
            ));
        }
        let limit = limit.min(self.config.check_timeout);
        let name = format!("Check_{:06}", self.next_check);
        self.next_check += 1;
        let theory = check::generate(&name, context, action)?;
        let started = Instant::now();
        // Isabelle reads a watchdog of 0 as "no limit", so a zero limit is not sent at all.
        let mut reading = if limit.is_zero() {
            check::read(&theory, &NodeReport::default())
                .undecided("the check's time limit is zero, so Isabelle was not asked".to_owned())
        } else {
            match self.run_theory(&theory, limit, started) {
                Ok(node) => check::read(&theory, &node),
                Err(PolicyError::Isabelle(message)) => check::read(&theory, &NodeReport::default())
                    .undecided(format!("Isabelle failed the check: {message}")),
                Err(error) => return Err(error),
            }
        };
        let elapsed = started.elapsed();
        if !limit.is_zero() && elapsed > limit {
            reading = reading.undecided(format!(
                "the check took {:.3}s, more than the {:.3}s limit",
                elapsed.as_secs_f64(),
                limit.as_secs_f64()
            ));
        }
        Ok(Check {
            proof_id: proof_id(&self.theory_sha256, &self.isabelle, &theory.text),
            theory: theory.name,
            theory_text: theory.text,
            theory_sha256: self.theory_sha256.clone(),
            isabelle: self.isabelle.clone(),
            context_digest: context_digest(context),
            action_digest: context.digest.clone(),
            verdict: reading.verdict,
            failing_rule: reading.failing_rule,
            rules: reading.rules,
            grants: reading.grants,
            authorizing_grants: reading.authorizing_grants,
            seconds: elapsed.as_secs_f64(),
        })
    }

    /// Has Isabelle process the check theory and end it at `limit`; both exchanges must be
    /// over by `started + limit + CHECK_GRACE`. `Err(PolicyError::Isabelle)` is Isabelle
    /// failing the check; any other error breaks the gate.
    fn run_theory(
        &mut self,
        theory: &CheckTheory,
        limit: Duration,
        started: Instant,
    ) -> Result<NodeReport, PolicyError> {
        let deadline = started + limit + CHECK_GRACE;
        let name = theory.name.as_str();
        // The file holds calendar data in plain text: it is deleted when `_file` goes out of
        // scope, however this function returns.
        let _file = match TheoryFile::write(&self.config.work_dir, name, &theory.text) {
            Ok(file) => file,
            Err(error) => {
                self.broken = true;
                return Err(error.into());
            }
        };
        let result = self.connection.task_until(
            "use_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir,
                "check_delay": CHECK_DELAY.as_secs_f64(),
                "check_limit": check_rounds(limit),
                "watchdog_timeout": limit.as_secs_f64()
            }),
            deadline,
        );
        let result = match result {
            Err(error) if !matches!(error, PolicyError::Isabelle(_)) => {
                // The answer may still arrive: nothing more is sent on this connection.
                self.broken = true;
                return Err(error);
            }
            result => result,
        };
        // A theory left behind only costs memory, so a failed purge does not fail the check;
        // but a purge whose answer did not arrive leaves it in the stream, which breaks the gate.
        let purge = self.connection.call_until(
            "purge_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir
            }),
            deadline,
        );
        if matches!(
            purge,
            Err(PolicyError::Io(_) | PolicyError::Protocol(_) | PolicyError::Json(_))
        ) {
            self.broken = true;
        }
        Ok(node_report(&result?, name))
    }

    /// Stops the gate within [`STOP_BOUND`] ([`BrokerGate::stop_within`]).
    pub fn stop(&mut self) -> Result<(), PolicyError> {
        self.stop_within(STOP_BOUND)
    }

    /// Stops the session, and the server too when this gate started it, within `bound`.
    ///
    /// On a server someone else started, the gate sends `session_stop` and ignores its
    /// result (on a fresh connection when this one is broken). A server this gate started
    /// gets `session_stop` and `shutdown` (only `shutdown`, on a fresh connection, when this
    /// one is broken). `Ok` means the server acknowledged the shutdown and its process exited
    /// within `bound`. If the process has not exited when `bound` is over, it is killed with
    /// everything it started ([`Server::kill`]) and the stop is an error. A stopped gate stops
    /// at once.
    ///
    /// `isabelle server` reuses a running server of the same name. The process this gate
    /// holds then exits at once and the kill cannot reach that server, so only the
    /// acknowledged shutdown says that it stopped. When that process had already exited as
    /// the stop began and the bounded exchange did not get `shutdown` acknowledged, the gate
    /// still delivers `shutdown` on a fresh connection, taking at most
    /// [`FOUND_SHUTDOWN_GRACE`] past `bound` and not waiting for the server to exit; the stop
    /// is then an error that says so.
    ///
    /// After [`BrokerGate::kill_handle`] was killed, the stop reaps the server
    /// ([`Server::kill`]) and is an error, at once. It says nothing to a server this gate
    /// started, which the kill ended. A server found already running was not reached by the
    /// kill, so it is sent `shutdown` without confirmation, as above.
    pub fn stop_within(&mut self, bound: Duration) -> Result<(), PolicyError> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        let deadline = Instant::now() + bound;
        let Some(server) = &mut self.server else {
            if !self.session_id.is_empty() {
                let argument = json!({"session_id": self.session_id});
                if self.broken {
                    if let Ok(mut connection) = fresh_connection(&self.address, deadline) {
                        let _ = connection.task_until("session_stop", &argument, deadline);
                    }
                } else {
                    let _ = self
                        .connection
                        .task_until("session_stop", &argument, deadline);
                }
            }
            return Ok(());
        };
        if self.kill.is_killed() {
            self.broken = true;
            let sent = self.found.then(|| {
                Connection::send_unanswered(
                    self.address.port,
                    &self.address.password,
                    "shutdown",
                    deadline + FOUND_SHUTDOWN_GRACE,
                )
            });
            // The wrapper of a found server is reaped already, and its port is not ours to
            // wait for.
            if !self.found {
                server.kill();
            }
            return Err(PolicyError::Isabelle(match sent {
                None => "the gate's Isabelle server was killed through its kill handle".to_owned(),
                Some(Ok(())) => "the gate's kill handle was killed, but the process this gate \
                                 started had exited before: its Isabelle server was found \
                                 already running and no kill reaches it; `shutdown` was sent \
                                 without confirmation on a fresh connection"
                    .to_owned(),
                Some(Err(error)) => format!(
                    "the gate's kill handle was killed, but the process this gate started had \
                     exited before: its Isabelle server was found already running, which no \
                     kill reaches, or has ended; `shutdown` could not be sent on a fresh \
                     connection ({error})"
                ),
            }));
        }
        // A wrapper that exited before the stop began: `isabelle server` found a server already
        // running (or the server ended by itself), and no kill can reach what serves the port.
        let found = self.found || server.has_exited();
        // The session is stopped first, as before: `shutdown` alone was measured to be no
        // faster. Its result is ignored, but a failed exchange leaves the connection unusable.
        if !self.broken && !self.session_id.is_empty() {
            let argument = json!({"session_id": self.session_id});
            if let Err(error) = self
                .connection
                .task_until("session_stop", &argument, deadline)
                && !matches!(error, PolicyError::Isabelle(_))
            {
                self.broken = true;
            }
        }
        let shutdown = if self.broken {
            fresh_connection(&self.address, deadline).and_then(|mut connection| {
                connection.call_until("shutdown", &Value::Null, deadline)
            })
        } else {
            self.connection
                .call_until("shutdown", &Value::Null, deadline)
        };
        if shutdown.is_err() {
            self.broken = true;
        }
        if found {
            return match shutdown {
                Ok(_) => Ok(()),
                Err(error) => {
                    let sent = Connection::send_unanswered(
                        self.address.port,
                        &self.address.password,
                        "shutdown",
                        deadline + FOUND_SHUTDOWN_GRACE,
                    );
                    let what = match sent {
                        Ok(()) => "`shutdown` was then sent without confirmation on a fresh \
                                   connection"
                            .to_owned(),
                        Err(again) => format!(
                            "`shutdown` could not be sent on a fresh connection either ({again})"
                        ),
                    };
                    Err(PolicyError::Isabelle(format!(
                        "the process this gate started had exited before the stop began: its \
                         Isabelle server was found already running, which no kill reaches, or \
                         has ended; it did not acknowledge `shutdown` within {:.3}s ({error}); \
                         {what}",
                        bound.as_secs_f64()
                    )))
                }
            };
        }
        loop {
            if server.has_exited() {
                return shutdown.map(|_| ());
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            thread::sleep(EXIT_POLL.min(deadline - now));
        }
        server.kill();
        Err(PolicyError::Isabelle(format!(
            "the Isabelle server did not stop within {:.3}s and was killed{}",
            bound.as_secs_f64(),
            shutdown
                .err()
                .map(|error| format!(" (shutdown: {error})"))
                .unwrap_or_default()
        )))
    }
}

/// Checks the session directory the gate gives Isabelle (`<rules_dir>/Fabric_Broker`) and
/// returns the SHA-256 of its `Fabric_Broker.thy` in hex. Isabelle reads `ROOT` and `ROOTS` of
/// every directory in `dirs`, so `ROOT` must equal [`SESSION_ROOT`] byte for byte, and nothing
/// named `ROOTS` (file, directory or symlink) may exist there. A violation is
/// [`PolicyError::Isabelle`] naming the path.
pub fn check_session_dir(session_dir: &Path) -> Result<String, PolicyError> {
    let theory = sha256_hex(&fs::read(session_dir.join("Fabric_Broker.thy"))?);
    let root = session_dir.join("ROOT");
    match fs::read(&root) {
        Ok(text) if text == SESSION_ROOT.as_bytes() => {}
        Ok(_) => {
            return Err(PolicyError::Isabelle(format!(
                "{} is not the session ROOT this crate was built with",
                root.display()
            )));
        }
        Err(error) => {
            return Err(PolicyError::Isabelle(format!(
                "cannot read {}: {error}",
                root.display()
            )));
        }
    }
    let roots = session_dir.join("ROOTS");
    match fs::symlink_metadata(&roots) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(theory),
        Ok(_) => Err(PolicyError::Isabelle(format!(
            "{} must not exist: Isabelle would read the sessions it names",
            roots.display()
        ))),
        Err(error) => Err(PolicyError::Isabelle(format!(
            "cannot tell whether {} exists: {error}",
            roots.display()
        ))),
    }
}

/// The number of looks at a running check after which Isabelle ends it (`check_limit` of
/// `use_theories`): the looks that fit into `limit`, and at least one, because Isabelle
/// reads 0 as "no limit".
#[must_use]
pub fn check_rounds(limit: Duration) -> u64 {
    let rounds = limit.as_millis().div_ceil(CHECK_DELAY.as_millis());
    u64::try_from(rounds).unwrap_or(u64::MAX).max(1)
}

/// A new connection to `address` that waits for the greeting no longer than until `deadline`.
fn fresh_connection(address: &ServerAddress, deadline: Instant) -> Result<Connection, PolicyError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(PolicyError::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "no time left to connect",
        )));
    }
    Connection::open(address.port, &address.password, left).map(|(connection, _)| connection)
}

/// Creates the work directory and removes the check theories a killed process left in it.
fn prepare_work_dir(work_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(work_dir)?;
    remove_stale_checks(work_dir)?;
    Ok(())
}

/// Removes every regular file named `Check_*.thy` directly in `dir`, and returns how many.
/// Such files hold calendar data in plain text, and a process killed during a check leaves
/// its theory behind.
pub fn remove_stale_checks(dir: &Path) -> io::Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let stale = name
            .to_str()
            .is_some_and(|name| name.starts_with("Check_") && name.ends_with(".thy"));
        if stale && entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Makes Isabelle keep its temporary files in `<ISABELLE_HOME_USER>/tmp` and returns that
/// directory. Without `USER` and `LOGNAME`, Isabelle's default prefix is `/tmp/isabelle-`,
/// shared by every account and created without an owner check.
///
/// `ISABELLE_HOME_USER` is `<user_home>/.isabelle/<identifier>`, with the identifier from
/// `<isabelle_home>/etc/ISABELLE_IDENTIFIER` (no file: `<user_home>/.isabelle`). Its
/// `etc/settings`, which Isabelle reads after its own, gets [`TMP_PREFIX_SETTING`] with a
/// comment unless it holds that line already; nothing else in it changes. The directory is
/// created if missing and its mode set to 0700. Running it again changes nothing.
#[cfg(unix)]
pub fn prepare_private_tmp(isabelle_home: &Path, user_home: &Path) -> io::Result<PathBuf> {
    use std::io::Write as _;
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

    let identifier = match fs::read_to_string(isabelle_home.join("etc").join("ISABELLE_IDENTIFIER"))
    {
        Ok(text) => text.trim().to_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut home_user = user_home.join(".isabelle");
    if !identifier.is_empty() {
        home_user.push(identifier);
    }
    let etc = home_user.join("etc");
    fs::create_dir_all(&etc)?;
    let settings = etc.join("settings");
    let existing = match fs::read_to_string(&settings) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if !existing
        .as_deref()
        .is_some_and(|text| text.lines().any(|line| line == TMP_PREFIX_SETTING))
    {
        let separator = match existing.as_deref() {
            Some(text) if !text.is_empty() && !text.ends_with('\n') => "\n",
            _ => "",
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&settings)?;
        write!(
            file,
            "{separator}# Fabric: Isabelle's temporary files stay in this directory, private to its owner.\n\
             {TMP_PREFIX_SETTING}\n"
        )?;
    }
    let tmp = home_user.join("tmp");
    match fs::symlink_metadata(&tmp) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::other(format!(
                "{} exists and is not a directory",
                tmp.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(&tmp)?;
        }
        Err(error) => return Err(error),
    }
    // An existing directory, and one made under a narrower umask, get the mode too.
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))?;
    Ok(tmp)
}

/// [`prepare_private_tmp`], then refuses to go on unless Isabelle, asked with the
/// environment the server gets, names that directory as its temporary prefix.
#[cfg(unix)]
fn require_private_tmp(config: &GateConfig) -> Result<(), PolicyError> {
    let tmp = prepare_private_tmp(&config.isabelle_home, &config.user_home)?;
    let prefix = fabric_policy::server::getenv(
        &config.isabelle_home,
        &config.user_home,
        "ISABELLE_TMP_PREFIX",
    )?;
    let same = match (fs::canonicalize(&prefix), fs::canonicalize(&tmp)) {
        (Ok(prefix), Ok(tmp)) => prefix == tmp,
        _ => false,
    };
    if same {
        Ok(())
    } else {
        Err(PolicyError::Isabelle(format!(
            "Isabelle would keep its temporary files in {prefix:?}, not in {}",
            tmp.display()
        )))
    }
}

/// A check theory's file in the work directory, which exists only while the guard does: it
/// is removed on drop, whatever the check's result, because it holds calendar data.
#[derive(Debug)]
pub struct TheoryFile {
    path: PathBuf,
}

impl TheoryFile {
    /// Writes `<dir>/<name>.thy`. A file left half-written by a failed write is removed.
    pub fn write(dir: &Path, name: &str, text: &str) -> std::io::Result<Self> {
        let file = TheoryFile {
            path: dir.join(format!("{name}.thy")),
        };
        fs::write(&file.path, text)?;
        Ok(file)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TheoryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for BrokerGate {
    fn drop(&mut self) {
        let _ = self.stop_within(DROP_STOP_BOUND);
    }
}

/// The node of `name` in a `use_theories` result, as [`NodeReport`]. A missing node is an
/// incomplete report.
#[must_use]
pub fn node_report(result: &Value, name: &str) -> NodeReport {
    let qualified = format!(".{name}");
    let Some(node) = result["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| {
            node["theory_name"]
                .as_str()
                .is_some_and(|theory| theory == name || theory.ends_with(&qualified))
        })
    else {
        return NodeReport::default();
    };
    let status = &node["status"];
    let mut report = NodeReport {
        complete: status["consolidated"].as_bool() == Some(true)
            && status["unprocessed"].as_u64() == Some(0)
            && status["running"].as_u64() == Some(0)
            && status["canceled"].as_bool() == Some(false),
        ..NodeReport::default()
    };
    for message in node["messages"].as_array().into_iter().flatten() {
        let line = message["pos"]["line"]
            .as_u64()
            .and_then(|line| usize::try_from(line).ok())
            .unwrap_or(0);
        let text = message["message"].as_str().unwrap_or("").to_owned();
        match message["kind"].as_str() {
            Some("error") => report.errors.push((line, text)),
            Some("writeln") => report.writeln.push((line, text)),
            _ => {}
        }
    }
    report
}
