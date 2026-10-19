//! T1.3 diagnostics: send one scripted turn to Claude with request variants and print the
//! response shape (stop reason and details, block types, usage) to find why plans degenerate.
//!
//!   target\debug\t1_claude_debug.exe <turn A-E> [--no-format] [--no-fallbacks] [--summarized] [--effort LEVEL]
//!
//! The API key and workspace ID are read from .env and never printed.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fabric_phase1_spikes::plans::plan_schema;
use fabric_phase1_spikes::{dotenv, turns};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let turn_id = arguments
        .first()
        .ok_or("usage: t1_claude_debug <turn> [flags]")?
        .clone();
    let flag = |name: &str| arguments.iter().any(|argument| argument == name);
    let effort = arguments
        .iter()
        .skip_while(|argument| *argument != "--effort")
        .nth(1)
        .cloned()
        .unwrap_or_else(|| "medium".to_owned());

    let values = dotenv::read(&PathBuf::from(".env"))?;
    let key = values
        .get("ANTHROPIC_API_KEY")
        .ok_or("ANTHROPIC_API_KEY missing")?;
    let workspace = values
        .get("ANTHROPIC_WORKSPACE_ID")
        .ok_or("ANTHROPIC_WORKSPACE_ID missing")?;

    let turn = turns::all()
        .into_iter()
        .find(|turn| turn.id == turn_id)
        .ok_or("unknown turn")?;
    let schema = plan_schema();
    let system = include_str!("../planner_system_prompt.txt");
    let mut user = format!(
        "Current state as JSON:\n{}\n\nReturn 2 or 3 alternative plans, best first, as JSON.",
        serde_json::to_string_pretty(&turn.state)?
    );
    if flag("--no-format") {
        user.push_str(&format!(
            "\n\nReply with only the JSON object, matching this schema:\n{}",
            serde_json::to_string(&schema)?
        ));
    }

    let mut output_config = json!({"effort": effort});
    if !flag("--no-format") {
        output_config["format"] = json!({"type": "json_schema", "schema": schema});
    }
    let mut thinking = json!({"type": "adaptive"});
    if flag("--summarized") {
        thinking["display"] = json!("summarized");
    }
    let mut body = json!({
        "model": "claude-opus-5-5",
        "max_tokens": 16000,
        "thinking": thinking,
        "output_config": output_config,
        "system": system,
        "messages": [{"role": "user", "content": user}]
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(600))
        .build();
    let mut request = agent
        .post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .set("anthropic-workspace-id", workspace);
    if !flag("--no-fallbacks") {
        body["fallbacks"] = json!("default");
        request = request.set("anthropic-beta", "server-side-fallback-2026-07-01");
    }

    let started = Instant::now();
    let value: Value = match request.send_json(body) {
        Ok(response) => response.into_json()?,
        Err(ureq::Error::Status(code, response)) => {
            println!(
                "HTTP {code}: {}",
                response.into_string().unwrap_or_default()
            );
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let seconds = started.elapsed().as_secs_f64();

    let directory = PathBuf::from(".local")
        .join("spikes")
        .join("t1_claude_debug");
    fs::create_dir_all(&directory)?;
    let label = arguments[1..].join("").replace("--", "_");
    fs::write(
        directory.join(format!("{turn_id}{label}.json")),
        serde_json::to_string_pretty(&value)?,
    )?;

    println!("turn {turn_id} flags {:?}: {seconds:.1}s", &arguments[1..]);
    println!(
        "  model {}  stop_reason {}",
        value["model"], value["stop_reason"]
    );
    println!("  stop_details {}", value["stop_details"]);
    println!("  usage {}", value["usage"]);
    for block in value["content"].as_array().into_iter().flatten() {
        let kind = block["type"].as_str().unwrap_or("?");
        let text = block["text"]
            .as_str()
            .or_else(|| block["thinking"].as_str())
            .unwrap_or("");
        let shown: String = text.chars().take(900).collect();
        println!("  [{kind}] {}", shown.replace('\n', " "));
        if kind == "fallback" {
            println!("    fallback block: {block}");
        }
    }
    Ok(())
}
