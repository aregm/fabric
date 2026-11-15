//! The trusted host's policy gate (section 5.2). It checks an agent's actions on Isabelle
//! against a context the host builds from its own records, and mints a token for each action an
//! authorize check permits, bound to the agent's registered key.
//!
//! Every verdict comes from Isabelle; the host has no other way to decide (D28).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use fabric_policy::{Action, CheckReport, Context, Gate, GateConfig, ServerAddress};
use fabric_token::{Minted, Minter, TokenError, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Where the host's gate finds Isabelle and the policy, from the per-run configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// The repository root, for `.tooling/isabelle` and `isabelle/Fabric_Policy`.
    pub repository: PathBuf,
    /// The shared Isabelle server that the launcher started (D9).
    pub isabelle: ServerAddress,
    /// Where this gate writes its check theories and its check log.
    pub work_dir: PathBuf,
}

/// A policy gate with its own session on the shared Isabelle server.
pub struct PolicyGate {
    gate: Arc<Mutex<Gate>>,
    minter: Minter,
}

impl PolicyGate {
    /// Opens a session on the shared server. This builds the policy session the first time and
    /// takes a few seconds, so hosts do it before they start serving.
    pub fn connect(config: &PolicyConfig, minter: Minter) -> Result<Self, String> {
        let mut gate_config = GateConfig::for_repository(&config.repository);
        gate_config.work_dir.clone_from(&config.work_dir);
        gate_config.audit_log = Some(config.work_dir.join("checks.jsonl"));
        let gate =
            Gate::connect(gate_config, &config.isabelle).map_err(|error| error.to_string())?;
        Ok(Self::new(gate, minter))
    }

    #[must_use]
    pub fn new(gate: Gate, minter: Minter) -> Self {
        Self {
            gate: Arc::new(Mutex::new(gate)),
            minter,
        }
    }

    /// Screens plans: one check theory for every action of every plan (D8).
    pub async fn screen(
        &self,
        context: Context,
        actions: Vec<Action>,
    ) -> Result<CheckReport, String> {
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || {
            gate.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .screen(&context, &actions)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    /// The execution-time check of one action (D2).
    pub async fn authorize(&self, context: Context, action: Action) -> Result<CheckReport, String> {
        let gate = self.gate.clone();
        tokio::task::spawn_blocking(move || {
            gate.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .authorize(&context, &action)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    /// Mints the token for a permitting authorize report.
    pub fn mint(
        &self,
        report: &CheckReport,
        holder: &VerifyingKey,
        now_ms: i64,
    ) -> Result<Minted, TokenError> {
        self.minter.mint(report, holder, now_ms)
    }

    /// Stops this gate's session; the shared server keeps running.
    pub fn stop(&self) {
        let _ = self
            .gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
    }
}
