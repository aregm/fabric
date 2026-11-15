//! The agent runtime (section 5.3, Phase 6): one per person, untrusted. It holds its planner's
//! credential and a token-binding key it makes at start, and nothing else: no calendar, no
//! other key, and no data beyond what its host and calendar service give it.

pub mod plan;
pub mod planner;
pub mod runtime;
pub mod turn;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fabric_token::BindingKey;
use serde::{Deserialize, Serialize};

pub use plan::{Plan, PlanAction, PlanSet, Slot, plan_schema};
pub use planner::{BackendConfig, Planned, Planner, Scripted};
pub use runtime::{Agent, Faults, HttpWorld, MAX_REPLANS, Outcome, World};
pub use turn::{Memory, SYSTEM_PROMPT, TurnInput};

use crate::audit::AuditLog;
use crate::calendar::CalendarClient;
use crate::clock::{Clock, ClockConfig, DemoClock};
use crate::host::HostClient;

/// One agent's settings, from the per-run configuration the launcher writes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub person: String,
    /// The base URLs of this person's trusted host and calendar service.
    pub host: String,
    pub calendar: String,
    pub agent_secret: String,
    pub backend: BackendConfig,
    pub clock: ClockConfig,
    #[serde(default)]
    pub audit_log: Option<PathBuf>,
    /// The git-ignored `.env` file with the Anthropic key, for the Claude backend.
    #[serde(default)]
    pub env_file: Option<PathBuf>,
}

impl AgentConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// Makes the agent's token-binding key, registers it with the host (section 5.3), and builds
/// the agent.
pub async fn start(config: &AgentConfig) -> Result<Agent<HttpWorld>, String> {
    let planner = Planner::from_config(&config.backend, config.env_file.as_deref())?;
    let key = BindingKey::generate().map_err(|error| error.to_string())?;
    let host = HostClient::new(&config.host, &config.agent_secret);
    host.register(&key.verifying_key())
        .await
        .map_err(|error| format!("registering with the host: {error}"))?;
    let clock: Arc<dyn Clock> = Arc::new(DemoClock::from_config(&config.clock)?);
    let audit = match &config.audit_log {
        Some(path) => {
            AuditLog::open(path).map_err(|error| format!("{}: {error}", path.display()))?
        }
        None => AuditLog::disabled(),
    };
    let world = HttpWorld {
        host,
        calendar: CalendarClient::new(&config.calendar),
        key,
        clock,
    };
    Ok(Agent::new(world, planner, &config.person, audit))
}
