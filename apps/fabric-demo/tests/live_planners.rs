//! One planning turn on each real backend (tasks T6.2 and T6.3). The Claude test spends about
//! 3 US cents and needs the key in `.env`; the Ollama test needs Ollama and `qwen3:4b`. Both are
//! local only:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-demo --test live_planners -- --ignored --nocapture

use std::path::Path;

use fabric_demo::agent::{BackendConfig, Memory, Planner};
use fabric_demo::host::{AgentState, NegotiationRecord, OwnerRequest};
use fabric_policy::time::parse_utc;
use fabric_policy::{Request, Role};

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

/// Alice at the start of the scenario: her request is filed and nothing else has happened.
fn alice_starts() -> AgentState {
    AgentState {
        person: "alice".to_owned(),
        peer: "bob".to_owned(),
        now_ms: at("2027-01-11T08:00Z"),
        request: Some(OwnerRequest {
            request: Request {
                peer: "bob".to_owned(),
                duration_ms: 30 * 60 * 1_000,
                window_start_ms: at("2027-01-11T08:00Z"),
                window_end_ms: at("2027-01-18T08:00Z"),
                opened_at_ms: at("2027-01-11T08:00Z"),
                expires_at_ms: at("2027-01-12T08:00Z"),
            },
            preference: "Weekday afternoons work best.".to_owned(),
        }),
        link: "idle".to_owned(),
        negotiation: NegotiationRecord::new(Role::Coordinator, Some("n-1".to_owned())),
        owner_decisions: Vec::new(),
    }
}

async fn first_turn(planner: Planner) {
    let input = Memory::default().turn_input(&alice_starts(), &[]);
    let planned = planner.plan(&input).await.expect("the planner answers");
    eprintln!(
        "{}: {} input and {} output tokens in {:.1}s\n{}",
        planned.model,
        planned.input_tokens,
        planned.output_tokens,
        planned.seconds,
        serde_json::to_string_pretty(&planned.plans).unwrap()
    );
    let plans = &planned.plans.plans;
    assert!((1..=3).contains(&plans.len()), "{} plans", plans.len());
    assert!(
        !plans[0].actions.is_empty(),
        "the first plan does something"
    );
    for plan in plans {
        for action in &plan.actions {
            action.to_action("n-1").expect("every action is readable");
        }
    }
}

#[tokio::test]
#[ignore = "calls the Claude API (about 3 US cents); run with --ignored"]
async fn claude_plans_a_first_turn() {
    let env = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    first_turn(Planner::from_config(&BackendConfig::Claude, Some(&env)).unwrap()).await;
}

#[tokio::test]
#[ignore = "needs Ollama with qwen3:4b; run with --ignored"]
async fn ollama_plans_a_first_turn() {
    let backend = BackendConfig::Ollama {
        model: "qwen3:4b".to_owned(),
    };
    first_turn(Planner::from_config(&backend, None).unwrap()).await;
}
