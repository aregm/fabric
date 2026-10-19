//! Planners (sections 10.2–10.4): Claude over raw HTTPS (both agents by default, Q27), Ollama
//! on this machine (optional, per run), and a scripted planner for automated tests only.

use std::collections::VecDeque;
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use fabric_policy::Decision;
use fabric_policy::time::{format_utc, parse_utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::plan::{Plan, PlanAction, PlanSet, Slot, plan_schema};
use super::turn::{SYSTEM_PROMPT, TurnInput};
use crate::env;

pub const CLAUDE_MODEL: &str = "claude-opus-5-5";
const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
const OLLAMA_URL: &str = "http://127.0.0.1:11434/api/chat";

/// Which planner an agent uses, from the per-run configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BackendConfig {
    Claude,
    Ollama {
        model: String,
    },
    /// Tests only; never the live demo.
    Scripted,
}

/// One planner reply.
#[derive(Clone, Debug)]
pub struct Planned {
    pub plans: PlanSet,
    /// The model that served the turn, which a fallback may change.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub seconds: f64,
}

#[derive(Debug)]
pub enum PlanError {
    /// The backend could not be reached or failed.
    Backend(String),
    /// The model declined or ran out of room (`refusal`, `max_tokens`).
    Stopped(String),
    /// The reply is not a plan set.
    Invalid(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(detail) => write!(f, "planner unavailable: {detail}"),
            Self::Stopped(reason) => write!(f, "planner stopped: {reason}"),
            Self::Invalid(detail) => {
                write!(f, "the plans are not valid JSON for the schema: {detail}")
            }
        }
    }
}

pub enum Planner {
    Claude(Claude),
    Ollama(Ollama),
    Scripted(Scripted),
}

impl Planner {
    pub fn from_config(config: &BackendConfig, env_file: Option<&Path>) -> Result<Self, String> {
        Ok(match config {
            BackendConfig::Claude => Self::Claude(Claude::from_environment(env_file)?),
            BackendConfig::Ollama { model } => Self::Ollama(Ollama::new(model)),
            BackendConfig::Scripted => Self::Scripted(Scripted::Rules),
        })
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Claude(_) => "claude",
            Self::Ollama(_) => "ollama",
            Self::Scripted(_) => "scripted",
        }
    }

    pub async fn plan(&self, input: &TurnInput) -> Result<Planned, PlanError> {
        match self {
            Self::Claude(claude) => claude.plan(input).await,
            Self::Ollama(ollama) => ollama.plan(input).await,
            Self::Scripted(scripted) => scripted.plan(input),
        }
    }
}

fn parse_plans(text: &str) -> Result<PlanSet, PlanError> {
    serde_json::from_str(text).map_err(|error| PlanError::Invalid(error.to_string()))
}

/// Claude over raw HTTPS (section 10.2); there is no official Anthropic SDK for Rust.
pub struct Claude {
    http: reqwest::Client,
    key: String,
    workspace: Option<String>,
}

impl fmt::Debug for Claude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Claude")
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

impl Claude {
    /// Reads `ANTHROPIC_API_KEY` and `ANTHROPIC_WORKSPACE_ID` from the environment, or else from
    /// the git-ignored `.env` file (Q25, Q26). The key is never printed or logged.
    pub fn from_environment(env_file: Option<&Path>) -> Result<Self, String> {
        let file = match env_file {
            Some(path) => {
                env::read(path).map_err(|error| format!("{}: {error}", path.display()))?
            }
            None => Default::default(),
        };
        let lookup = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .or_else(|| file.get(name).cloned().filter(|value| !value.is_empty()))
        };
        let key = lookup("ANTHROPIC_API_KEY")
            .ok_or("ANTHROPIC_API_KEY is in neither the environment nor the .env file")?;
        Ok(Self {
            http: reqwest::Client::new(),
            key,
            workspace: lookup("ANTHROPIC_WORKSPACE_ID"),
        })
    }

    async fn plan(&self, input: &TurnInput) -> Result<Planned, PlanError> {
        let started = Instant::now();
        let turn = serde_json::to_string_pretty(input)
            .map_err(|error| PlanError::Invalid(error.to_string()))?;
        let body = json!({
            "model": CLAUDE_MODEL,
            "max_tokens": 16_000,
            "thinking": {"type": "adaptive"},
            "output_config": {
                "effort": "medium",
                "format": {"type": "json_schema", "schema": plan_schema()}
            },
            "fallbacks": "default",
            "system": SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": turn}]
        });
        let mut request = self
            .http
            .post(CLAUDE_URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "server-side-fallback-2026-07-01")
            .header("content-type", "application/json")
            .timeout(Duration::from_secs(300));
        if let Some(workspace) = &self.workspace {
            request = request.header("anthropic-workspace-id", workspace);
        }
        let response = request
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| PlanError::Backend(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| PlanError::Backend(error.to_string()))?;
        if !status.is_success() {
            return Err(PlanError::Backend(format!("HTTP {status}: {text}")));
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|error| PlanError::Invalid(error.to_string()))?;
        let stop_reason = value["stop_reason"].as_str().unwrap_or_default();
        if matches!(stop_reason, "refusal" | "max_tokens") {
            return Err(PlanError::Stopped(stop_reason.to_owned()));
        }
        let answer = value["content"]
            .as_array()
            .and_then(|blocks| blocks.iter().find(|block| block["type"] == "text"))
            .and_then(|block| block["text"].as_str())
            .ok_or_else(|| PlanError::Invalid("the reply has no text block".to_owned()))?;
        Ok(Planned {
            plans: parse_plans(answer)?,
            model: value["model"].as_str().unwrap_or(CLAUDE_MODEL).to_owned(),
            input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

/// A local model through Ollama (section 10.3).
#[derive(Debug)]
pub struct Ollama {
    http: reqwest::Client,
    model: String,
}

impl Ollama {
    #[must_use]
    pub fn new(model: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            model: model.to_owned(),
        }
    }

    async fn plan(&self, input: &TurnInput) -> Result<Planned, PlanError> {
        let started = Instant::now();
        let schema = plan_schema();
        let turn = serde_json::to_string_pretty(input)
            .map_err(|error| PlanError::Invalid(error.to_string()))?;
        // Ollama's documentation recommends repeating the schema in the prompt.
        let system =
            format!("{SYSTEM_PROMPT}\n\nAnswer with JSON that matches this schema:\n{schema}");
        let body = json!({
            "model": self.model,
            "stream": false,
            "think": false,
            "format": schema,
            "options": {"temperature": 0, "num_ctx": 8192},
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": turn}
            ]
        });
        let response = self
            .http
            .post(OLLAMA_URL)
            .header("content-type", "application/json")
            .body(body.to_string())
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|error| PlanError::Backend(error.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| PlanError::Backend(error.to_string()))?;
        if !status.is_success() {
            return Err(PlanError::Backend(format!("HTTP {status}: {text}")));
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|error| PlanError::Invalid(error.to_string()))?;
        Ok(Planned {
            plans: parse_plans(value["message"]["content"].as_str().unwrap_or_default())?,
            model: format!("ollama:{}", self.model),
            input_tokens: value["prompt_eval_count"].as_u64().unwrap_or(0),
            output_tokens: value["eval_count"].as_u64().unwrap_or(0),
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

/// The scripted planner (section 10.4), for automated tests only.
pub enum Scripted {
    /// Returns these plan sets in order, one per turn, then waits.
    Fixed(Mutex<VecDeque<PlanSet>>),
    /// Follows the section 14 scenario by fixed rules: read, call, propose the first three free
    /// working-hours slots, answer from free/busy, decide as the owner did, and add the meeting
    /// both owners accepted.
    Rules,
}

impl Scripted {
    #[must_use]
    pub fn fixed(plans: Vec<PlanSet>) -> Self {
        Self::Fixed(Mutex::new(plans.into()))
    }

    fn plan(&self, input: &TurnInput) -> Result<Planned, PlanError> {
        let plans = match self {
            Self::Fixed(queue) => queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| single("wait", "the script has ended", Vec::new())),
            Self::Rules => rules(input),
        };
        Ok(Planned {
            plans,
            model: "scripted".to_owned(),
            input_tokens: 0,
            output_tokens: 0,
            seconds: 0.0,
        })
    }
}

fn single(label: &str, rationale: &str, actions: Vec<PlanAction>) -> PlanSet {
    PlanSet {
        plans: vec![Plan {
            label: label.to_owned(),
            rationale: rationale.to_owned(),
            actions,
        }],
    }
}

fn wait(why: &str) -> PlanSet {
    single("wait", why, Vec::new())
}

fn done(input: &TurnInput, kind: &str) -> bool {
    input.your_past_actions.iter().any(|action| {
        action["kind"] == kind
            && action["round"].as_u64() == Some(u64::from(input.negotiation.round))
    })
}

fn rules(input: &TurnInput) -> PlanSet {
    let Some(request) = &input.owner_request else {
        return wait("no request yet");
    };
    let negotiation = &input.negotiation;
    let coordinator = input.your_role.starts_with("coordinator");
    let read_window = || PlanAction::ReadFreeBusy {
        start: request.window.start.clone(),
        end: request.window.end.clone(),
    };
    let handshake = || PlanAction::Handshake {
        peer: request.meet_with.clone(),
    };

    // Both owners accepted an option: add it, then the coordinator closes. The invitee may add
    // the meeting after the coordinator has already closed.
    let accepted: Vec<&String> = negotiation
        .mutual_options
        .iter()
        .filter(|id| {
            let yes = |decisions: &[Decision]| {
                decisions
                    .iter()
                    .any(|decision| &decision.candidate_id == *id && decision.accept)
            };
            yes(&negotiation.your_decisions_sent) && yes(&negotiation.decisions_from_peer)
        })
        .collect();
    if let Some(id) = accepted.first() {
        let added = input
            .your_past_actions
            .iter()
            .any(|action| action["kind"] == "add_meeting");
        let close = PlanAction::SendClose {
            reason: fabric_policy::CloseReason::Agreed,
            note: "see you then".to_owned(),
        };
        let open = negotiation.closed.is_none();
        if added {
            return if coordinator && open {
                single("close", "the meeting is in the calendar", vec![close])
            } else {
                wait("the meeting is in the calendar")
            };
        }
        let Some(slot) = negotiation.candidates.iter().find(|slot| &slot.id == *id) else {
            return wait("the accepted option is unknown");
        };
        let meeting = PlanAction::AddMeeting {
            start: slot.start.clone(),
            minutes: slot.minutes,
        };
        let actions = if coordinator && open {
            vec![meeting, close]
        } else {
            vec![meeting]
        };
        return single("add", "both owners said yes", actions);
    }

    if negotiation.closed.is_some() {
        return wait("the negotiation is closed");
    }
    if negotiation.session == "ringing" {
        return single("answer", "the other agent is calling", vec![handshake()]);
    }
    if input.known_free_busy.is_none() && (coordinator || !negotiation.candidates.is_empty()) {
        return single("read", "free/busy comes first", vec![read_window()]);
    }

    // The owner decided on every mutual option: send the decisions as given.
    if !negotiation.mutual_options.is_empty()
        && negotiation.your_decisions_sent.is_empty()
        && !negotiation.your_owner_decisions.is_empty()
    {
        let decisions: Vec<Decision> = negotiation
            .mutual_options
            .iter()
            .filter_map(|id| {
                negotiation
                    .your_owner_decisions
                    .iter()
                    .find(|decision| &decision.candidate_id == id)
                    .cloned()
            })
            .collect();
        if decisions.len() == negotiation.mutual_options.len() {
            let decide = PlanAction::SendDecide {
                round: negotiation.round,
                decisions,
                note: String::new(),
            };
            return single("decide", "the owner decided", vec![decide]);
        }
        return wait("the owner has not decided on every option");
    }

    if coordinator {
        if negotiation.round > 0 {
            return wait("waiting for the other person");
        }
        if negotiation.session != "connected" {
            if negotiation.session == "calling" {
                return wait("waiting for the call to be answered");
            }
            return single("call", "a session comes first", vec![handshake()]);
        }
        let Some(free_busy) = &input.known_free_busy else {
            return single("read", "free/busy comes first", vec![read_window()]);
        };
        let slots = free_slots(free_busy, request.duration_minutes, 3);
        if slots.is_empty() {
            let close = PlanAction::SendClose {
                reason: fabric_policy::CloseReason::NoMatch,
                note: "no free time".to_owned(),
            };
            return single("close", "no free working-hours slot", vec![close]);
        }
        let propose = PlanAction::SendPropose {
            round: 1,
            candidates: slots,
            note: "afternoons suit me".to_owned(),
        };
        return single(
            "propose",
            "the first free working-hours slots",
            vec![propose],
        );
    }

    // The invitee answers each received candidate from its owner's free/busy.
    if !negotiation.candidates.is_empty() && !done(input, "send_respond") {
        let Some(free_busy) = &input.known_free_busy else {
            return single("read", "free/busy comes first", vec![read_window()]);
        };
        let answers = negotiation
            .candidates
            .iter()
            .map(|slot| fabric_policy::Answer {
                candidate_id: slot.id.clone(),
                available: is_free(free_busy, &slot.start, slot.minutes),
            })
            .collect();
        let respond = PlanAction::SendRespond {
            round: negotiation.round,
            answers,
            note: String::new(),
        };
        return single("respond", "answers from free/busy", vec![respond]);
    }
    wait("waiting for the other person")
}

const MINUTE_MS: i64 = 60 * 1_000;
const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

fn is_free(free_busy: &super::turn::FreeBusyView, start: &str, minutes: i64) -> bool {
    let Some(start) = parse_utc(start) else {
        return false;
    };
    let end = start + minutes * MINUTE_MS;
    free_busy.busy.iter().all(
        |busy| match (parse_utc(&busy.start), parse_utc(&busy.end)) {
            (Some(busy_start), Some(busy_end)) => end <= busy_start || busy_end <= start,
            _ => false,
        },
    )
}

/// The first `count` free slots of `minutes` on the 15-minute grid, in weekday working hours
/// (09:00 to 17:00 UTC), after the start of the known window.
fn free_slots(free_busy: &super::turn::FreeBusyView, minutes: i64, count: usize) -> Vec<Slot> {
    let (Some(start), Some(end)) = (
        parse_utc(&free_busy.window.start),
        parse_utc(&free_busy.window.end),
    ) else {
        return Vec::new();
    };
    let step = 15 * MINUTE_MS;
    let length = minutes * MINUTE_MS;
    let mut slot = (start.div_euclid(step) + 1) * step;
    let mut found = Vec::new();
    while slot + length <= end && found.len() < count {
        let weekday = (slot.div_euclid(DAY_MS) + 4) % 7;
        let minute = slot.rem_euclid(DAY_MS) / MINUTE_MS;
        let working = (1..=5).contains(&weekday)
            && minute >= 9 * 60
            && minute * MINUTE_MS + length <= 17 * 60 * MINUTE_MS;
        let start_text = format_utc(slot);
        if working && is_free(free_busy, &start_text, minutes) {
            found.push(Slot {
                start: start_text,
                minutes,
            });
        }
        slot += step;
    }
    found
}
