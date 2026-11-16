//! The broker's proof gate: it renders the trusted context and one action into a check
//! theory, has a running Isabelle server prove every rule or its negation and the outcome,
//! and returns the verdict with its proof id (plan section 8). It is modelled on
//! [`fabric_policy::Gate`] and uses the same server and connection.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fabric_policy::PolicyError;
use fabric_policy::server::{Connection, Server, ServerAddress, ServerOptions, platform_path};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::check::{self, GrantResult, NodeReport, RuleResult, Verdict};
use crate::model::{Action, Context, Rule};
use crate::report::{context_digest, proof_id, sha256_hex};

/// The Isabelle session of the rule theory (G0-THY).
pub const SESSION: &str = "Fabric_Broker";
const SESSION_START_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub struct GateConfig {
    pub isabelle_home: PathBuf,
    pub user_home: PathBuf,
    /// The directory with `ROOTS` and `Fabric_Broker/`.
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

impl GateConfig {
    /// Defaults for a repository checkout set up with `scripts/setup-demo.sh`.
    #[must_use]
    pub fn for_repository(root: &Path) -> Self {
        let isabelle = root.join(".tooling").join("isabelle");
        Self {
            isabelle_home: isabelle.join("Isabelle2025-2"),
            user_home: isabelle.join("home"),
            rules_dir: root.join("isabelle"),
            work_dir: root.join(".local").join("broker-checks"),
            server_name: "fabric-broker".to_owned(),
            check_timeout: Duration::from_secs(30),
            consolidate_delay_seconds: 0.1,
        }
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

/// A running Isabelle session with the rule theory loaded.
pub struct BrokerGate {
    config: GateConfig,
    /// The server, when this gate started it.
    server: Option<Server>,
    connection: Connection,
    session_id: String,
    master_dir: String,
    theory_sha256: String,
    isabelle: String,
    next_check: u64,
    stopped: bool,
}

impl BrokerGate {
    /// Starts the Isabelle server and opens a `Fabric_Broker` session, building its heap if
    /// needed. Stopping the gate stops the server.
    pub fn start(config: GateConfig) -> Result<Self, PolicyError> {
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
        fs::create_dir_all(&config.work_dir)?;
        let mut server = Server::start(&ServerOptions {
            isabelle_home: &config.isabelle_home,
            user_home: &config.user_home,
            name: &config.server_name,
            log_dir: &config.work_dir,
        })?;
        match Connection::open(server.port, &server.password, SESSION_START_TIMEOUT) {
            Ok((connection, greeting)) => Self::open(config, Some(server), connection, &greeting),
            Err(error) => {
                server.wait_or_kill(Duration::ZERO);
                Err(error)
            }
        }
    }

    /// Connects to a server someone else started and opens a session on it. Stopping the
    /// gate stops only that session.
    pub fn connect(config: GateConfig, address: &ServerAddress) -> Result<Self, PolicyError> {
        fs::create_dir_all(&config.work_dir)?;
        let (connection, greeting) =
            Connection::open(address.port, &address.password, SESSION_START_TIMEOUT)?;
        Self::open(config, None, connection, &greeting)
    }

    fn open(
        config: GateConfig,
        server: Option<Server>,
        connection: Connection,
        greeting: &Value,
    ) -> Result<Self, PolicyError> {
        let theory = fs::read(config.rules_dir.join(SESSION).join("Fabric_Broker.thy"))?;
        let isabelle = greeting["isabelle_name"]
            .as_str()
            .map_or_else(|| greeting.to_string(), str::to_owned);
        let master_dir = platform_path(&config.work_dir);
        let mut gate = Self {
            config,
            server,
            connection,
            session_id: String::new(),
            master_dir,
            theory_sha256: sha256_hex(&theory),
            isabelle,
            next_check: 1,
            stopped: false,
        };
        if let Err(error) = gate.open_session() {
            let _ = gate.stop();
            return Err(error);
        }
        Ok(gate)
    }

    fn open_session(&mut self) -> Result<(), PolicyError> {
        let session = self.connection.task(
            "session_start",
            &json!({
                "session": SESSION,
                "dirs": [platform_path(&self.config.rules_dir)],
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

    /// Checks one action in one context. A context or action that cannot be rendered is an
    /// error; a check Isabelle did not finish, or finished out of time, is undecided.
    pub fn check(&mut self, context: &Context, action: &Action) -> Result<Check, PolicyError> {
        let name = format!("Check_{:06}", self.next_check);
        self.next_check += 1;
        let theory = check::generate(&name, context, action)?;
        let started = Instant::now();
        let node = self.run_theory(&theory.name, &theory.text);
        let elapsed = started.elapsed();
        let mut reading = match node {
            Ok(node) => check::read(&theory, &node),
            Err(PolicyError::Isabelle(message)) => check::read(&theory, &NodeReport::default())
                .undecided(format!("Isabelle failed the check: {message}")),
            Err(error) => return Err(error),
        };
        if elapsed > self.config.check_timeout {
            reading = reading.undecided(format!(
                "the check took {:.3}s, more than the {:.3}s limit",
                elapsed.as_secs_f64(),
                self.config.check_timeout.as_secs_f64()
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

    fn run_theory(&mut self, name: &str, text: &str) -> Result<NodeReport, PolicyError> {
        // The file holds calendar data in plain text: it is deleted when `_file` goes out of
        // scope, however this function returns.
        let _file = TheoryFile::write(&self.config.work_dir, name, text)?;
        let result = self.connection.task(
            "use_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir,
                "check_delay": 0.05,
                "watchdog_timeout": self.config.check_timeout.as_secs_f64()
            }),
        );
        // A theory left behind only costs memory, so a failed purge does not fail the check.
        let _ = self.connection.call(
            "purge_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir
            }),
        );
        Ok(node_report(&result?, name))
    }

    /// Stops the session, and the server too when this gate started it. The result of
    /// `session_stop` is ignored and an owned server always gets `shutdown`, as in
    /// [`fabric_policy::Gate::stop`].
    pub fn stop(&mut self) -> Result<(), PolicyError> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        if !self.session_id.is_empty() {
            let _ = self
                .connection
                .task("session_stop", &json!({"session_id": self.session_id}));
        }
        let Some(server) = &mut self.server else {
            return Ok(());
        };
        let shutdown = self.connection.call("shutdown", &Value::Null);
        server.wait_or_kill(Duration::from_secs(20));
        shutdown.map(|_| ())
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
        let _ = self.stop();
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
