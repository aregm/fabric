//! A token minted from a live Isabelle check. It needs `scripts/setup-demo.ps1`, so it is
//! ignored by default:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-token --test live_gate -- --ignored --nocapture

use std::path::Path;

use fabric_policy::time::parse_utc;
use fabric_policy::{Action, Context, Gate, GateConfig, Negotiation, Request, Role};
use fabric_token::{
    BindingKey, MemoryStore, PersonKeys, Reason, ReplayCache, Service, TokenError, Verifier, hex,
};

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
}

/// Alice coordinates with Bob within 7 days, before any proposal.
fn alice() -> Context {
    Context {
        owner: "alice".to_owned(),
        now_ms: at("2027-01-11T08:00Z"),
        request: Request {
            peer: "bob".to_owned(),
            duration_ms: 30 * 60 * 1_000,
            window_start_ms: at("2027-01-11T08:00Z"),
            window_end_ms: at("2027-01-18T08:00Z"),
            opened_at_ms: at("2027-01-11T08:00Z"),
            expires_at_ms: at("2027-01-12T08:00Z"),
        },
        negotiation: Negotiation {
            id: "n1".to_owned(),
            role: Role::Coordinator,
            round: 0,
            handshake_done: false,
            candidates: Vec::new(),
            mutual: Vec::new(),
        },
        consents: Vec::new(),
    }
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
fn an_isabelle_permit_mints_a_token_the_calendar_accepts() {
    let mut config = GateConfig::for_repository(repository());
    config.work_dir = repository().join(".local").join("token-live");
    config.server_name = "fabric-token-live".to_owned();
    let mut gate = Gate::start(config).expect("the gate starts");

    let context = alice();
    let keys = PersonKeys::load_or_create(&MemoryStore::new(), "alice").unwrap();
    let minter = keys.minter();
    let agent = BindingKey::generate().unwrap();

    let read = Action::ReadFreeBusy {
        start_ms: at("2027-01-11T08:00Z"),
        end_ms: at("2027-01-18T08:00Z"),
    };
    let report = gate.authorize(&context, &read).expect("the check runs");
    assert!(report.verdicts[0].verdict.is_permit(), "{report:?}");
    let minted = minter
        .mint(&report, &agent.verifying_key(), context.now_ms)
        .unwrap();
    assert_eq!(hex::encode(&minted.claims.proof), report.proof_id);
    eprintln!(
        "{} permitted in {:.2}s; token of {} bytes: {}",
        report.theory,
        report.seconds,
        minted.bytes.len(),
        serde_json::to_string(&minted.claims).unwrap()
    );

    let calendar = Verifier::new(Service::Calendar, "alice", keys.public().gate().unwrap());
    let replay = ReplayCache::new();
    let body = serde_json::to_vec(&read).unwrap();
    let presentation = agent.present(&minted.bytes, &body, context.now_ms).unwrap();
    calendar
        .check(&presentation, &read, &body, context.now_ms, &replay)
        .expect("the calendar accepts the token")
        .commit();
    let replayed = calendar
        .check(&presentation, &read, &body, context.now_ms, &replay)
        .expect_err("the token works once");
    assert_eq!(replayed.reason, Reason::TokenReplayed);

    // An over-broad read is denied, and no token can be minted from the denial.
    let broad = Action::ReadFreeBusy {
        start_ms: at("2027-01-11T08:00Z"),
        end_ms: at("2027-02-11T08:00Z"),
    };
    let denied = gate.authorize(&context, &broad).expect("the check runs");
    assert!(!denied.verdicts[0].verdict.is_permit(), "{denied:?}");
    eprintln!("{}", denied.verdicts[0].verdict.summary());
    assert!(matches!(
        minter.mint(&denied, &agent.verifying_key(), context.now_ms),
        Err(TokenError::Mint(_))
    ));

    gate.stop().expect("the gate stops");
}
