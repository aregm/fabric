//! T1.2 and T1.3: run the five scripted turns against a planner backend with the plan schema.
//!
//! Run from the repository root:
//!   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo run --manifest-path spikes/phase1/Cargo.toml --bin t1_planners -- ollama:qwen3:4b 2
//!   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo run --manifest-path spikes/phase1/Cargo.toml --bin t1_planners -- claude 1
//!
//! The API key and workspace ID are read from .env and never printed.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fabric_phase1_spikes::plans::{PlanSet, meets, plan_schema, precheck};
use fabric_phase1_spikes::{dotenv, turns};
use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn Error>>;

const CLAUDE_MODEL: &str = "claude-opus-5-5";
const CLAUDE_INPUT_PER_MTOK: f64 = 4.0;
const CLAUDE_OUTPUT_PER_MTOK: f64 = 20.0;

const SYSTEM_PROMPT: &str = include_str!("../planner_system_prompt.txt");

enum Backend {
    Claude { key: String, workspace: String },
    Ollama { model: String },
}

struct Call {
    text: String,
    input_tokens: u64,
    output_tokens: u64,
    served_model: String,
    stop_reason: String,
}

fn main() -> Res<()> {
    let arguments: Vec<String> = std::env::args().collect();
    let selector = arguments
        .get(1)
        .ok_or("usage: t1_planners <claude | ollama:MODEL> [repetitions]")?;
    let repetitions: usize = arguments.get(2).map_or(Ok(1), |text| text.parse())?;
    let only_turns: Option<&String> = arguments.get(3);
    let backend = if selector == "claude" {
        let values = dotenv::read(&PathBuf::from(".env"))?;
        Backend::Claude {
            key: values
                .get("ANTHROPIC_API_KEY")
                .cloned()
                .ok_or("ANTHROPIC_API_KEY missing from .env")?,
            workspace: values
                .get("ANTHROPIC_WORKSPACE_ID")
                .cloned()
                .ok_or("ANTHROPIC_WORKSPACE_ID missing from .env")?,
        }
    } else if let Some(model) = selector.strip_prefix("ollama:") {
        Backend::Ollama {
            model: model.to_owned(),
        }
    } else {
        return Err(format!("unknown backend {selector}").into());
    };

    let output = PathBuf::from(".local")
        .join("spikes")
        .join("t1_planners")
        .join(selector.replace(':', "_"));
    fs::create_dir_all(&output)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(900))
        .build();
    let schema = plan_schema();

    let mut rows = Vec::new();
    for turn in turns::all() {
        if only_turns.is_some_and(|ids| !ids.contains(turn.id)) {
            continue;
        }
        for repetition in 0..repetitions {
            let mut user = format!(
                "Current state as JSON:\n{}\n\nReturn 2 or 3 alternative plans, best first, as JSON.",
                serde_json::to_string_pretty(&turn.state)?
            );
            if matches!(backend, Backend::Ollama { .. }) {
                user.push_str(&format!(
                    "\n\nThe JSON must match this schema:\n{}",
                    serde_json::to_string(&schema)?
                ));
            }

            let started = Instant::now();
            let call = match &backend {
                Backend::Claude { key, workspace } => {
                    call_claude(&agent, key, workspace, &schema, &user)
                }
                Backend::Ollama { model } => call_ollama(&agent, model, &schema, &user),
            };
            let seconds = started.elapsed().as_secs_f64();

            let mut row = Row {
                turn: turn.id,
                repetition,
                seconds,
                ..Row::default()
            };
            match call {
                Err(error) => row.error = Some(error),
                Ok(call) => {
                    fs::write(
                        output.join(format!("{}-{repetition}.json", turn.id)),
                        &call.text,
                    )?;
                    row.input_tokens = call.input_tokens;
                    row.output_tokens = call.output_tokens;
                    row.served_model = call.served_model;
                    row.stop_reason = call.stop_reason;
                    match serde_json::from_str::<PlanSet>(&call.text) {
                        Err(error) => row.error = Some(format!("schema: {error}")),
                        Ok(set) => {
                            row.valid = true;
                            row.plans = set.plans.len();
                            let checked: Vec<Vec<String>> = set
                                .plans
                                .iter()
                                .map(|plan| precheck(&turn.context, plan))
                                .collect();
                            row.permitted = checked
                                .iter()
                                .filter(|violations| violations.is_empty())
                                .count();
                            row.first_plan_violations =
                                checked.first().cloned().unwrap_or_default();
                            row.chosen =
                                checked.iter().position(|violations| violations.is_empty());
                            row.correct = row.chosen.is_some_and(|index| {
                                meets(&turn.expectation, &turn.context, &set.plans[index])
                            });
                            row.first_plan_kinds = set
                                .plans
                                .first()
                                .map(|plan| {
                                    plan.actions
                                        .iter()
                                        .map(|action| action.kind())
                                        .collect::<Vec<_>>()
                                        .join(" > ")
                                })
                                .unwrap_or_default();
                        }
                    }
                }
            }
            println!("{}", row.line(turn.summary));
            rows.push(row);
        }
    }
    summarize(selector, &rows);
    Ok(())
}

#[derive(Default)]
struct Row {
    turn: &'static str,
    repetition: usize,
    seconds: f64,
    valid: bool,
    plans: usize,
    permitted: usize,
    chosen: Option<usize>,
    correct: bool,
    first_plan_kinds: String,
    first_plan_violations: Vec<String>,
    input_tokens: u64,
    output_tokens: u64,
    served_model: String,
    stop_reason: String,
    error: Option<String>,
}

impl Row {
    fn line(&self, summary: &str) -> String {
        if let Some(error) = &self.error {
            return format!(
                "{}#{} {:>6.1}s  ERROR {}  ({summary})",
                self.turn,
                self.repetition,
                self.seconds,
                error.chars().take(300).collect::<String>()
            );
        }
        format!(
            "{}#{} {:>6.1}s  plans={} permitted={} chosen={} correct={}  tokens in={} out={}  stop={}  first plan: {}{}",
            self.turn,
            self.repetition,
            self.seconds,
            self.plans,
            self.permitted,
            self.chosen
                .map_or("none".to_owned(), |index| (index + 1).to_string()),
            if self.correct { "yes" } else { "NO" },
            self.input_tokens,
            self.output_tokens,
            self.stop_reason,
            self.first_plan_kinds,
            if self.first_plan_violations.is_empty() {
                String::new()
            } else {
                format!(
                    "  [first-plan pre-check: {}]",
                    self.first_plan_violations.join("; ")
                )
            }
        )
    }
}

fn summarize(selector: &str, rows: &[Row]) {
    let count = rows.len() as f64;
    let valid = rows.iter().filter(|row| row.valid).count();
    let correct = rows.iter().filter(|row| row.correct).count();
    let all_permitted = rows
        .iter()
        .filter(|row| row.valid && row.permitted == row.plans)
        .count();
    let mean = rows.iter().map(|row| row.seconds).sum::<f64>() / count;
    let max = rows.iter().map(|row| row.seconds).fold(0.0, f64::max);
    let input: u64 = rows.iter().map(|row| row.input_tokens).sum();
    let output: u64 = rows.iter().map(|row| row.output_tokens).sum();
    let served: Vec<&str> = {
        let mut models: Vec<&str> = rows
            .iter()
            .map(|row| row.served_model.as_str())
            .filter(|m| !m.is_empty())
            .collect();
        models.sort_unstable();
        models.dedup();
        models
    };
    println!("\n== {selector}: {} calls", rows.len());
    println!("  schema-valid: {valid}/{}", rows.len());
    println!(
        "  runtime would pick a correct plan: {correct}/{}",
        rows.len()
    );
    println!(
        "  every plan passed the pre-check: {all_permitted}/{}",
        rows.len()
    );
    println!("  latency: mean {mean:.1}s, max {max:.1}s");
    println!(
        "  tokens: input {input}, output {output}; served by {}",
        served.join(", ")
    );
    if selector == "claude" {
        let cost = input as f64 / 1e6 * CLAUDE_INPUT_PER_MTOK
            + output as f64 / 1e6 * CLAUDE_OUTPUT_PER_MTOK;
        println!(
            "  cost at ${CLAUDE_INPUT_PER_MTOK}/${CLAUDE_OUTPUT_PER_MTOK} per MTok: ${cost:.3} total, ${:.3} per call",
            cost / count
        );
    }
}

fn call_claude(
    agent: &ureq::Agent,
    key: &str,
    workspace: &str,
    schema: &Value,
    user: &str,
) -> Result<Call, String> {
    let body = json!({
        "model": CLAUDE_MODEL,
        "max_tokens": 16000,
        "thinking": {"type": "adaptive"},
        "output_config": {"effort": "medium", "format": {"type": "json_schema", "schema": schema}},
        "fallbacks": "default",
        "system": SYSTEM_PROMPT,
        "messages": [{"role": "user", "content": user}]
    });
    let response = agent
        .post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .set("anthropic-beta", "server-side-fallback-2026-07-01")
        .set("anthropic-workspace-id", workspace)
        .send_json(body);
    let value: Value = match response {
        Ok(response) => response.into_json().map_err(|error| error.to_string())?,
        Err(ureq::Error::Status(code, response)) => {
            return Err(format!(
                "HTTP {code}: {}",
                response.into_string().unwrap_or_default()
            ));
        }
        Err(error) => return Err(error.to_string()),
    };
    let stop_reason = value["stop_reason"].as_str().unwrap_or("").to_owned();
    if stop_reason == "refusal" || stop_reason == "max_tokens" {
        return Err(format!("stop_reason {stop_reason}"));
    }
    let text = value["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|block| block["type"] == "text"))
        .and_then(|block| block["text"].as_str())
        .unwrap_or("")
        .to_owned();
    Ok(Call {
        text,
        input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
        served_model: value["model"].as_str().unwrap_or("").to_owned(),
        stop_reason,
    })
}

fn call_ollama(
    agent: &ureq::Agent,
    model: &str,
    schema: &Value,
    user: &str,
) -> Result<Call, String> {
    let body = json!({
        "model": model,
        "stream": false,
        "think": false,
        "format": schema,
        "options": {"temperature": 0, "num_ctx": 8192},
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user}
        ]
    });
    let response = agent
        .post("http://127.0.0.1:11434/api/chat")
        .send_json(body);
    let value: Value = match response {
        Ok(response) => response.into_json().map_err(|error| error.to_string())?,
        Err(ureq::Error::Status(code, response)) => {
            return Err(format!(
                "HTTP {code}: {}",
                response.into_string().unwrap_or_default()
            ));
        }
        Err(error) => return Err(error.to_string()),
    };
    Ok(Call {
        text: value["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        input_tokens: value["prompt_eval_count"].as_u64().unwrap_or(0),
        output_tokens: value["eval_count"].as_u64().unwrap_or(0),
        served_model: model.to_owned(),
        stop_reason: value["done_reason"].as_str().unwrap_or("").to_owned(),
    })
}
