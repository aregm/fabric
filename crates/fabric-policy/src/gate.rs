//! The policy gate (specification section 5.2): it renders the trusted context and an agent's
//! actions into a check theory, has Isabelle prove each rule or its negation, and returns
//! verdicts with a proof ID for tokens and the audit log.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::check::{self, ActionVerdict, NodeReport};
use crate::error::PolicyError;
use crate::model::{Action, Context, Parameters};
use crate::server::{Connection, Server, ServerAddress, ServerOptions, platform_path};

const SESSION: &str = "Fabric_Policy";
const SESSION_START_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub struct GateConfig {
    pub isabelle_home: PathBuf,
    pub user_home: PathBuf,
    /// The directory with `ROOTS` and `Fabric_Policy/`.
    pub policy_dir: PathBuf,
    /// Where check theories and server logs are written.
    pub work_dir: PathBuf,
    pub server_name: String,
    /// Isabelle's watchdog for one check (specification section 8.3: 30 seconds).
    pub check_timeout: Duration,
    /// `headless_consolidate_delay` for the session (gate G1: 0.1 seconds).
    pub consolidate_delay_seconds: f64,
    /// Appends every check report as one JSON line when set.
    pub audit_log: Option<PathBuf>,
}

impl GateConfig {
    /// Defaults for a repository checkout set up with `scripts/setup-demo.ps1`.
    #[must_use]
    pub fn for_repository(root: &Path) -> Self {
        let isabelle = root.join(".tooling").join("isabelle");
        Self {
            isabelle_home: isabelle.join("Isabelle2025-2"),
            user_home: isabelle.join("home"),
            policy_dir: root.join("isabelle"),
            work_dir: root.join(".local").join("policy-checks"),
            server_name: "fabric-policy".to_owned(),
            check_timeout: Duration::from_secs(30),
            consolidate_delay_seconds: 0.1,
            audit_log: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// Plan screening: advisory, mints no token.
    Screen,
    /// The check right before an action runs; a permit here may mint a token.
    Authorize,
}

/// Everything about one check, for tokens, the dashboard's "view proof", and the audit log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckReport {
    pub kind: CheckKind,
    pub theory: String,
    /// SHA-256 over the check theory, the policy theory's hash and the Isabelle version.
    pub proof_id: String,
    pub policy_sha256: String,
    pub isabelle: String,
    pub seconds: f64,
    pub verdicts: Vec<ActionVerdict>,
    pub theory_text: String,
}

/// A running Isabelle session with the policy loaded.
pub struct Gate {
    config: GateConfig,
    /// The server, when this gate started it.
    server: Option<Server>,
    connection: Connection,
    session_id: String,
    master_dir: String,
    policy_sha256: String,
    isabelle: String,
    parameters: Parameters,
    next_check: u64,
    stopped: bool,
}

impl Gate {
    /// Starts the Isabelle server, opens a `Fabric_Policy` session (building it if needed),
    /// and reads the policy parameters from the theory. Stopping the gate stops the server.
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

    /// Connects to a server someone else started, and opens a session on it. Stopping the gate
    /// stops only that session, so other gates on the server keep working (D9).
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
        let policy = fs::read(config.policy_dir.join(SESSION).join("Fabric_Policy.thy"))?;
        let policy_sha256 = hex(Sha256::digest(&policy).as_slice());
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
            policy_sha256,
            isabelle,
            parameters: Parameters::DEFAULT,
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
                "dirs": [platform_path(&self.config.policy_dir)],
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
        self.parameters = self.read_parameters()?;
        Ok(())
    }

    /// The parameters the policy theory defines, as read from Isabelle at start.
    #[must_use]
    pub const fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// The SHA-256 of `Fabric_Policy.thy` as loaded.
    #[must_use]
    pub fn policy_sha256(&self) -> &str {
        &self.policy_sha256
    }

    /// Checks every action of every alternative plan in one theory (decision D8).
    pub fn screen(
        &mut self,
        context: &Context,
        actions: &[Action],
    ) -> Result<CheckReport, PolicyError> {
        self.check(CheckKind::Screen, context, actions)
    }

    /// Checks one action right before it runs (decision D2).
    pub fn authorize(
        &mut self,
        context: &Context,
        action: &Action,
    ) -> Result<CheckReport, PolicyError> {
        self.check(CheckKind::Authorize, context, std::slice::from_ref(action))
    }

    fn check(
        &mut self,
        kind: CheckKind,
        context: &Context,
        actions: &[Action],
    ) -> Result<CheckReport, PolicyError> {
        let name = format!("Check_{:06}", self.next_check);
        self.next_check += 1;
        let theory = check::generate(&name, context, actions)?;
        let started = Instant::now();
        let node = self.run_theory(&theory.name, &theory.text)?;
        let seconds = started.elapsed().as_secs_f64();
        let verdicts = check::verdicts(&theory, &node, context, actions, &self.parameters);
        let report = CheckReport {
            kind,
            proof_id: proof_id(&self.policy_sha256, &self.isabelle, &theory.text),
            theory: theory.name,
            policy_sha256: self.policy_sha256.clone(),
            isabelle: self.isabelle.clone(),
            seconds,
            verdicts,
            theory_text: theory.text,
        };
        if let Some(path) = &self.config.audit_log {
            append_line(path, &serde_json::to_string(&report)?)?;
        }
        Ok(report)
    }

    fn run_theory(&mut self, name: &str, text: &str) -> Result<NodeReport, PolicyError> {
        fs::write(self.config.work_dir.join(format!("{name}.thy")), text)?;
        let result = self.connection.task(
            "use_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir,
                "check_delay": 0.05,
                "watchdog_timeout": self.config.check_timeout.as_secs_f64()
            }),
        )?;
        // A theory left behind only costs memory, so a failed purge does not fail the check.
        let _ = self.connection.call(
            "purge_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir
            }),
        );
        Ok(node_report(&result, name))
    }

    fn read_parameters(&mut self) -> Result<Parameters, PolicyError> {
        let names = [
            "max_candidates",
            "max_rounds",
            "horizon_ms",
            "grid_ms",
            "consent_ttl_ms",
            "note_max",
        ];
        let mut lines = vec![
            "theory Policy_Parameters".to_owned(),
            format!("  imports \"{}\"", check::POLICY_THEORY),
            "begin".to_owned(),
        ];
        lines.extend(names.iter().map(|name| format!("value \"{name}\"")));
        lines.push("end".to_owned());
        let node = self.run_theory("Policy_Parameters", &(lines.join("\n") + "\n"))?;
        let value = |index: usize| {
            let line = index + 4;
            node.writeln
                .iter()
                .find(|(at, _)| *at == line)
                .and_then(|(_, text)| check::parse_number(text))
                .ok_or_else(|| {
                    PolicyError::Isabelle(format!("no value for parameter {}", names[index]))
                })
        };
        let signed = |index: usize| {
            value(index).and_then(|number| {
                i64::try_from(number).map_err(|_| {
                    PolicyError::Isabelle(format!("parameter {} is out of range", names[index]))
                })
            })
        };
        Ok(Parameters {
            max_candidates: value(0)?,
            max_rounds: value(1)?,
            horizon_ms: signed(2)?,
            grid_ms: signed(3)?,
            consent_ttl_ms: signed(4)?,
            note_max: value(5)?,
        })
    }

    /// Stops the session, and the server too when this gate started it. `session_stop` always
    /// reports return code 130 on Windows, so its result is ignored and an owned server always
    /// gets `shutdown` (gate G1).
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

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn node_report(result: &Value, name: &str) -> NodeReport {
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

fn proof_id(policy_sha256: &str, isabelle: &str, theory: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fabric-policy-proof-v1\n");
    hasher.update(policy_sha256.as_bytes());
    hasher.update(b"\n");
    hasher.update(isabelle.as_bytes());
    hasher.update(b"\n");
    hasher.update(theory.as_bytes());
    hex(hasher.finalize().as_slice())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn append_line(path: &Path, line: &str) -> Result<(), PolicyError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}
