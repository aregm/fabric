//! Unit tests of the owner channel over a real store (design 9.1, C): the broker on a test-key
//! store in a temporary directory, driven by owner lines. The refusal, `hello` and
//! deduplication tests come from the CD-W02 stand-in broker.

use fabric_desktop_contracts::agent::AgentRequest;
use fabric_desktop_contracts::error::{ErrorCode, Fault};
use fabric_desktop_contracts::owner::{
    Account, Mode, OwnerResponse, ProfileRows, ProofReport, ProverProblem, Ruling,
};
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{
    Entry, Frame, Limits, MAX_FRAME_BYTES, canonical, decode, expansion_digest,
};
use fabric_storage::{Counters, ProverKind, Store};
use serde_json::{Value, json};

use super::profiles::expansion;
use crate::broker::{Attachments, Broker, Input, Job, Output, Seed, SeedAgent, Settings};
use crate::ports::{
    AgentInput, AttemptDone, AttemptReport, ConnectionId, ProofDone, ProofJob, ProverIdentity,
    ProverStatus,
};
use crate::service::core::testing::{self, T0, TempDir};

const AGENT: &str = "agent-a";
const ACCOUNT: &str = "acct-1";
const CALENDAR: &str = "cal-1";
const BUILD: &str = "0.1.0 CD-W04 test";

fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}

fn record<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    fabric_desktop_contracts::decode_record(serde_json::to_string(value).unwrap().as_bytes())
        .unwrap()
}

fn account(account_id: &str) -> Account {
    record(
        &json!({"account_id": account_id, "revision": 1, "state": "connected",
        "email": "owner@example.com",
        "calendars": [{"calendar_id": CALENDAR, "name": "Calendar", "access": "owner",
                       "selected": true, "agent_drafts": false}]}),
    )
}

fn synthetic() -> Attachments {
    Attachments {
        prover: Some(ProverIdentity {
            kind: ProverKind::Synthetic,
            health: ProverStatus::Unavailable(Some(ProverProblem::NotConfigured)),
        }),
        provider_present: false,
    }
}

fn owner_line(frame_id: &str, method: &str, params: Value) -> Vec<u8> {
    canonical(
        &json!({"protocol": "fabric-desktop/1", "id": frame_id, "method": method,
                      "params": params}),
    )
    .into_bytes()
}

/// Checks that every owner frame decodes again as what it is.
fn checked(output: &Output) {
    for frame in &output.owner {
        let line = frame.to_line();
        let entry = match frame {
            Frame::OwnerResponse(_) => Entry::OwnerResponse,
            Frame::OwnerNotice(_) => Entry::OwnerNotice,
            other => panic!("the broker wrote {other:?}"),
        };
        let back = decode(entry, &line.as_bytes()[..line.len() - 1])
            .unwrap_or_else(|r| panic!("{r}: {line}"));
        assert_eq!(&back, frame);
    }
}

/// The only response in `output`, as JSON.
fn response(output: &Output) -> Value {
    let responses: Vec<&Frame> = output
        .owner
        .iter()
        .filter(|f| matches!(f, Frame::OwnerResponse(_)))
        .collect();
    assert_eq!(responses.len(), 1, "{:?}", output.owner);
    responses[0].to_value()
}

fn response_line(output: &Output) -> String {
    output
        .owner
        .iter()
        .find(|f| matches!(f, Frame::OwnerResponse(_)))
        .expect("a response")
        .to_line()
}

fn result(response: &Value) -> Value {
    response
        .get("result")
        .unwrap_or_else(|| panic!("expected a result: {response}"))
        .clone()
}

fn error(response: &Value) -> Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("expected an error: {response}"))
        .clone()
}

/// The notices in `output` before its response, as JSON.
fn notices_before_response(output: &Output) -> Vec<Value> {
    output
        .owner
        .iter()
        .take_while(|f| matches!(f, Frame::OwnerNotice(_)))
        .map(Frame::to_value)
        .collect()
}

fn jobs(output: &Output) -> Vec<ProofJob> {
    output
        .jobs
        .iter()
        .filter_map(|j| match j {
            Job::Prove(p) => Some(p.clone()),
            Job::Dispatch(_) => None,
        })
        .collect()
}

/// A result for `job` named `proof_id`.
fn done(job: &ProofJob, outcome: &str, proof_id: &str) -> ProofDone {
    let rule = matches!(outcome, "block" | "need_approval").then_some("test_rule");
    let report: ProofReport = record(&json!({
        "proof_id": proof_id, "request_id": job.request_id, "purpose": job.purpose,
        "outcome": outcome, "failing_rule": rule, "rule_verdicts": [],
        "isabelle_version": "Isabelle2025",
        "theory_hash": format!("sha256:{}", "a".repeat(64)),
        "context_digest": format!("sha256:{}", "b".repeat(64)),
        "action_digest": job.action_digest,
        "started_at": crate::format_timestamp(job.started_at_ms),
        "elapsed_ms": 10
    }));
    let ruling = rule.map(|rule| {
        record::<Ruling>(&json!({"rule": rule, "explanation": "Because.", "proof_id": proof_id}))
    });
    ProofDone {
        job_id: job.job_id,
        request_id: job.request_id.clone(),
        purpose: job.purpose,
        report: Some(report),
        ruling,
        authorizing_grants: Vec::new(),
        undecided_reason: None,
        check_theory_text: None,
        cause: None,
    }
}

/// A broker run in a test: the broker, its directory and the raw clock.
struct Run {
    dir: TempDir,
    attachments: Attachments,
    broker: Option<Broker>,
    now: u64,
    frames: u64,
}

impl Run {
    fn new(attachments: Attachments) -> Run {
        let mut run = Run {
            dir: TempDir::new(),
            attachments,
            broker: None,
            now: T0,
            frames: 0,
        };
        run.start();
        run
    }

    fn start(&mut self) {
        let (broker, _) = Broker::start(
            testing::opener(&self.dir),
            Settings::cd_w04(BUILD.to_owned()),
            self.attachments.clone(),
            self.now,
        )
        .unwrap_or_else(|e| panic!("the broker does not start: {e}"));
        self.broker = Some(broker);
    }

    /// Drops the broker without quit, then starts it again (a kill and a restart).
    fn restart(&mut self) {
        self.broker = None;
        self.now += 1_000;
        self.start();
    }

    /// Stops the broker and reads the store it left.
    fn store<T>(&mut self, f: impl FnOnce(&Store) -> T) -> T {
        self.broker = None;
        let store = (testing::opener(&self.dir))(i64::try_from(self.now).unwrap()).unwrap();
        store.verify_chain().unwrap();
        store.verify_replay().unwrap();
        let value = f(&store);
        store.close().unwrap();
        value
    }

    fn counters(&mut self) -> Counters {
        let counters = self.store(|s| s.read(|r| r.counters()).unwrap());
        self.start();
        counters
    }

    fn handle(&mut self, input: Input) -> Output {
        let now = self.now;
        let output = self.broker.as_mut().expect("a broker").handle(input, now);
        assert!(output.halt.is_none(), "the step halted: {:?}", output.halt);
        checked(&output);
        output
    }

    fn line(&mut self, line: Vec<u8>) -> Output {
        self.handle(Input::OwnerLine(line))
    }

    fn send(&mut self, frame_id: &str, method: &str, params: Value) -> Output {
        self.line(owner_line(frame_id, method, params))
    }

    /// One owner request with a fresh id; its response and the whole output.
    fn owner(&mut self, method: &str, params: Value) -> (Value, Output) {
        self.frames += 1;
        let frame_id = format!("frame-{}", self.frames);
        let output = self.send(&frame_id, method, params);
        (response(&output), output)
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        result(&self.owner(method, params).0)
    }

    fn refused(&mut self, method: &str, params: Value) -> Value {
        error(&self.owner(method, params).0)
    }

    fn snapshot(&mut self) -> Value {
        self.ok("get_snapshot", json!({}))
    }

    fn details(&mut self, request_id: &str) -> Value {
        self.ok(
            "get_request_details",
            json!({"request_id": request_id, "redaction": "redacted"}),
        )
    }

    fn seed(&mut self) {
        let seed = Seed {
            agents: vec![SeedAgent {
                agent_id: id(AGENT),
                registered_name: Label::new(AGENT).unwrap(),
                destination: fabric_desktop_contracts::owner::Destination::Local {
                    endpoint: Label::new("in-process").unwrap(),
                },
            }],
            accounts: vec![account(ACCOUNT)],
        };
        let now = self.now;
        let _ = self.broker.as_mut().expect("a broker").seed(&seed, now);
        let _ = self.handle(Input::Agent(AgentInput::Connect {
            connection: ConnectionId(1),
            agent_id: id(AGENT),
        }));
    }

    /// A `propose_create`; its request id and the output.
    fn propose(&mut self, call_id: &str) -> (String, Output) {
        self.propose_on(call_id, CALENDAR)
    }

    /// A `propose_create` on `calendar` of the account; its request id and the output.
    fn propose_on(&mut self, call_id: &str, calendar: &str) -> (String, Output) {
        let line = canonical(&json!({"protocol": "fabric-desktop/1", "id": call_id,
            "method": "fabric_calendar_propose_create",
            "params": {"account_id": ACCOUNT, "calendar_id": calendar,
                "event": {"title": "Plan",
                    "start": {"date_time": "2026-11-27T10:00:00+01:00", "zone": "Europe/Berlin"},
                    "end": {"date_time": "2026-11-27T11:00:00+01:00", "zone": "Europe/Berlin"},
                    "description": null, "location": null},
                "task": "Plan the week."}}));
        let Ok(Frame::AgentRequest(request)) = decode(Entry::AgentRequest, line.as_bytes()) else {
            panic!("not an agent request");
        };
        let request: AgentRequest = request;
        let output = self.handle(Input::Agent(AgentInput::Call {
            connection: ConnectionId(1),
            request,
        }));
        let reply = output.agent.last().expect("a reply").frame.to_value();
        let request_id = reply["result"]["request"]["request_id"]
            .as_str()
            .unwrap_or_else(|| panic!("no request: {reply}"))
            .to_owned();
        (request_id, output)
    }

    fn prove(&mut self, job: &ProofJob, outcome: &str, proof_id: &str) -> Output {
        self.handle(Input::ProofDone(done(job, outcome, proof_id)))
    }

    /// A proposal the classification sends to the owner with proof `proof_id`.
    fn pending(&mut self, call_id: &str, proof_id: &str) -> String {
        let (request_id, output) = self.propose(call_id);
        let job = jobs(&output).pop().expect("a classification job");
        let _ = self.prove(&job, "need_approval", proof_id);
        assert_eq!(self.state(&request_id), "pending_approval");
        request_id
    }

    fn state(&mut self, request_id: &str) -> String {
        self.details(request_id)["request"]["status"]["state"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn decide_params(&mut self, request_id: &str, decision: &str, duration: Value) -> Value {
        let request = self.details(request_id)["request"].clone();
        json!({"request_id": request_id, "request_digest": request["request_digest"],
               "expected_revision": request["revision"], "decision": decision,
               "selected": null, "duration": duration})
    }

    /// Approves a pending request; the decision's output.
    fn approve(&mut self, request_id: &str, duration: &str) -> Output {
        let params = self.decide_params(request_id, "approve", json!({"kind": duration}));
        self.owner("decide_request", params).1
    }

    fn subscribe(&mut self) -> u64 {
        let cursor = self.snapshot()["cursor"].as_u64().unwrap();
        let _ = self.ok("subscribe", json!({"cursor": cursor}));
        cursor
    }
}

fn hello_params(schema_version: u64) -> Value {
    json!({"client": {"name": "test", "build": "1"}, "schema_version": schema_version})
}

fn error_parts(response: &Value) -> (Value, Value, Value, Value) {
    let error = error(response);
    (
        response["id"].clone(),
        response["method"].clone(),
        error["code"].clone(),
        error["detail"].clone(),
    )
}

#[test]
fn hello_reports_the_broker_the_build_and_the_limits() {
    let mut run = Run::new(testing::isabelle());
    let answer = run.ok("hello", hello_params(2));
    assert_eq!(answer["broker"]["name"], json!("fabric-broker"));
    assert_eq!(answer["broker"]["build"], json!(BUILD));
    assert_eq!(answer["schema_version"], json!(2));
    assert_eq!(answer["capabilities"], json!([]));
    assert_eq!(
        answer["limits"],
        serde_json::to_value(Limits::CD_G0).unwrap()
    );
}

#[test]
fn hello_with_another_schema_version_is_refused() {
    let mut run = Run::new(testing::isabelle());
    let output = run.send("h", "hello", hello_params(1));
    assert_eq!(
        error_parts(&response(&output)),
        (
            json!("h"),
            json!("hello"),
            json!("unsupported_schema_version"),
            json!({"received": 1, "supported": 2})
        )
    );
}

#[test]
fn refusals_by_layer() {
    let mut run = Run::new(testing::isabelle());
    let mut answer = |line: &[u8]| error_parts(&response(&run.line(line.to_vec())));
    let malformed = (
        Value::Null,
        Value::Null,
        json!("malformed_frame"),
        Value::Null,
    );
    assert_eq!(answer(b"{not json"), malformed);
    assert_eq!(answer(b"[]"), malformed);
    assert_eq!(answer(b""), malformed);
    assert_eq!(answer(&vec![b' '; MAX_FRAME_BYTES + 1]), malformed);
    assert_eq!(
        answer(br#"{"protocol":"fabric-desktop/2","id":"x","method":"hello","params":{}}"#),
        (
            json!("x"),
            json!("hello"),
            json!("unsupported_protocol"),
            json!({"received": "fabric-desktop/2", "supported": ["fabric-desktop/1"]})
        )
    );
    let long = format!(
        r#"{{"protocol":"{}","id":"-bad","method":7}}"#,
        "p".repeat(201)
    );
    assert_eq!(
        answer(long.as_bytes()),
        (
            Value::Null,
            Value::Null,
            json!("unsupported_protocol"),
            json!({"received": null, "supported": ["fabric-desktop/1"]})
        )
    );
    assert_eq!(
        answer(br#"{"protocol":"fabric-desktop/1","id":"x","method":"teleport","params":{}}"#),
        (
            json!("x"),
            json!("teleport"),
            json!("unknown_method"),
            Value::Null
        )
    );
    assert_eq!(
        answer(br#"{"protocol":"fabric-desktop/1","id":"x","method":"hello","params":{}}"#),
        (
            json!("x"),
            json!("hello"),
            json!("invalid_params"),
            json!({"pointer": "/params"})
        )
    );
}

#[test]
fn an_oversized_line_is_malformed() {
    let mut run = Run::new(testing::isabelle());
    let output = run.handle(Input::OwnerOversized);
    assert_eq!(
        error_parts(&response(&output)),
        (
            Value::Null,
            Value::Null,
            json!("malformed_frame"),
            Value::Null
        )
    );
}

#[test]
fn connect_google_is_unknown_in_this_build_and_not_stored() {
    let mut run = Run::new(testing::isabelle());
    let output = run.send("connect-1", "connect_google", json!({"account_id": null}));
    let OwnerResponse::Error { error, .. } = (match &output.owner[..] {
        [Frame::OwnerResponse(r)] => r.clone(),
        other => panic!("{other:?}"),
    }) else {
        panic!("expected an error");
    };
    assert_eq!(error.code(), ErrorCode::UnknownMethod);
    assert_eq!(error.fault.detail(), Value::Null);
    assert_eq!(
        error.message.as_str(),
        "This build cannot connect a Google account yet; CD-W05 adds it."
    );
    assert_eq!(
        run.store(|s| s.read(|r| r.command(&id("connect-1"))).unwrap()),
        None
    );
}

#[test]
fn a_duplicate_command_gets_the_same_bytes_and_changes_nothing() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let params = run.decide_params(&request_id, "approve", json!({"kind": "until_revoked"}));
    let first = run.send("cmd-1", "decide_request", params.clone());
    assert!(result(&response(&first))["approval"].is_object());
    let before = run.snapshot();
    let _ = run.subscribe();

    let again = run.send("cmd-1", "decide_request", params.clone());
    assert_eq!(
        again.owner.len(),
        1,
        "a repeat sent notices: {:?}",
        again.owner
    );
    assert_eq!(response_line(&again), response_line(&first));
    let mut after = run.snapshot();
    // The pump of the first decision may have moved the request on; nothing else changed.
    after["cursor"] = before["cursor"].clone();
    after["requests"] = before["requests"].clone();
    assert_eq!(after, before);
    assert_eq!(run.snapshot()["grants"].as_array().map(Vec::len), Some(1));

    // After a restart the stored answer is the same, byte for byte.
    run.restart();
    let again = run.send("cmd-1", "decide_request", params);
    assert_eq!(response_line(&again), response_line(&first));
    let commands = run.store(|s| s.read(|r| r.command(&id("cmd-1"))).unwrap());
    assert_eq!(
        commands.map(|c| c.method),
        Some("decide_request".to_owned())
    );
}

#[test]
fn a_duplicate_refusal_is_answered_again() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let mut params = run.decide_params(&request_id, "deny", Value::Null);
    params["expected_revision"] = json!(1);
    let first = run.send("cmd-1", "decide_request", params.clone());
    assert_eq!(error(&response(&first))["code"], json!("stale_request"));
    let again = run.send("cmd-1", "decide_request", params);
    assert_eq!(response_line(&again), response_line(&first));
    assert_eq!(run.state(&request_id), "pending_approval");
}

#[test]
fn a_reused_command_id_is_refused() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let approve = run.decide_params(&request_id, "approve", json!({"kind": "one_request"}));
    let _ = run.send("cmd-1", "decide_request", approve);
    let deny = json!({"request_id": request_id, "request_digest": format!("sha256:{}", "0".repeat(64)),
        "expected_revision": 9, "decision": "deny", "selected": null, "duration": null});
    let output = run.send("cmd-1", "decide_request", deny);
    assert_eq!(
        error_parts(&response(&output)),
        (
            json!("cmd-1"),
            json!("decide_request"),
            json!("command_reused"),
            Value::Null
        )
    );
    // The audit row is checked by the acceptance test c16 (the store has no audit reader).
    assert_ne!(run.state(&request_id), "denied");
}

#[test]
fn stale_unknown_and_narrowed_decisions_are_refused() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let base = run.decide_params(&request_id, "deny", Value::Null);
    let mut stale_revision = base.clone();
    stale_revision["expected_revision"] = json!(9);
    let mut stale_digest = base.clone();
    stale_digest["request_digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    let mut unknown = base.clone();
    unknown["request_id"] = json!("request-7");
    let mut narrowed = run.decide_params(&request_id, "approve", json!({"kind": "one_request"}));
    narrowed["selected"] = run.details(&request_id)["card"]["scope"].clone();
    let cases = [
        (stale_revision, "stale_request", Value::Null),
        (stale_digest, "stale_request", Value::Null),
        (unknown, "not_found", Value::Null),
        (
            narrowed,
            "invalid_params",
            json!({"pointer": "/params/selected"}),
        ),
    ];
    for (n, (params, code, detail)) in cases.into_iter().enumerate() {
        let command_id = format!("cmd-{n}");
        let output = run.send(&command_id, "decide_request", params);
        assert_eq!(
            error_parts(&response(&output)),
            (
                json!(command_id),
                json!("decide_request"),
                json!(code),
                detail
            )
        );
    }
    assert_eq!(run.state(&request_id), "pending_approval");
    assert_eq!(run.snapshot()["grants"], json!([]));
    // A request that is no longer pending is stale too.
    let _ = run.send("deny", "decide_request", base.clone());
    let output = run.send("deny-again", "decide_request", base);
    assert_eq!(error(&response(&output))["code"], json!("stale_request"));
}

fn custom(name: &str, profile_id: Value, rows: ProfileRows, acknowledged: Value) -> Value {
    json!({"kind": "save_custom",
           "profile": {"profile_id": profile_id, "name": name, "rows": rows},
           "acknowledged_expansion": acknowledged})
}

fn save_profile(run: &mut Run, revision: u64, change: Value) -> (Value, Output) {
    run.owner(
        "save_profile",
        json!({"expected_revision": revision, "change": change}),
    )
}

/// The active profile id and version, from the store.
fn active(run: &mut Run) -> (String, u64) {
    let (id, version, _) = run.store(|s| s.read(|r| r.active_profile()).unwrap());
    run.start();
    (id.as_str().to_owned(), version)
}

#[test]
fn activation_raises_the_profiles_revision_and_the_revocation_epoch_once() {
    let mut run = Run::new(testing::isabelle());
    let epoch = run.counters().revocation_epoch;
    let cursor = run.subscribe();
    let (response, output) = save_profile(
        &mut run,
        0,
        json!({"kind": "activate", "profile_id": "flexible"}),
    );
    let answer = result(&response);
    assert_eq!(answer["profiles"]["revision"], json!(1));
    assert_eq!(answer["profiles"]["active_profile_id"], json!("flexible"));
    let sent = notices_before_response(&output);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["change"]["kind"], json!("profiles"));
    assert_eq!(answer["commit_sequence"], json!(cursor + 1));
    assert_eq!(answer["commit_sequence"], sent[0]["sequence"]);
    assert_eq!(run.counters().revocation_epoch, epoch + 1);

    let refusal = error(
        &save_profile(
            &mut run,
            1,
            json!({"kind": "activate", "profile_id": "custom-0"}),
        )
        .0,
    );
    assert_eq!(refusal["code"], json!("not_found"));
    assert_eq!(run.snapshot()["profiles"]["revision"], json!(1));
}

#[test]
fn a_custom_profile_named_like_a_built_in_is_refused() {
    let mut run = Run::new(testing::isabelle());
    for name in ["Careful", "flexible", "CAREFUL"] {
        let refusal = error(
            &save_profile(
                &mut run,
                0,
                custom(name, Value::Null, ProfileRows::CAREFUL, Value::Null),
            )
            .0,
        );
        assert_eq!(refusal["code"], json!("invalid_params"), "{name}");
        assert_eq!(
            refusal["detail"]["pointer"],
            json!("/params/change/profile/name")
        );
    }
    assert_eq!(run.snapshot()["profiles"]["revision"], json!(0));
}

#[test]
fn an_expansion_needs_the_owner_to_acknowledge_its_digest() {
    let mut run = Run::new(testing::isabelle());
    let change = custom("Team", Value::Null, ProfileRows::FLEXIBLE, Value::Null);
    let refusal = error(&save_profile(&mut run, 0, change).0);
    assert_eq!(refusal["code"], json!("expansion_not_acknowledged"));
    let expected = expansion(&ProfileRows::CAREFUL, &ProfileRows::FLEXIBLE).unwrap();
    let digest = expansion_digest(&expected);
    assert_eq!(
        refusal["detail"]["expansion"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(refusal["detail"]["expansion_digest"], json!(digest));

    let wrong = format!("sha256:{}", "0".repeat(64));
    let change = custom("Team", Value::Null, ProfileRows::FLEXIBLE, json!(wrong));
    let refusal = error(&save_profile(&mut run, 0, change).0);
    assert_eq!(refusal["code"], json!("expansion_not_acknowledged"));

    let change = custom("Team", Value::Null, ProfileRows::FLEXIBLE, json!(digest));
    let answer = result(&save_profile(&mut run, 0, change).0);
    let saved = answer["profiles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == json!("custom"))
        .cloned()
        .expect("the custom profile");
    assert_eq!(saved["version"], json!(1));
    assert!(saved["profile_id"].as_str().unwrap().starts_with("custom-"));
}

#[test]
fn saving_a_custom_profile_raises_the_revision_and_only_the_active_one_the_epoch() {
    let mut run = Run::new(testing::isabelle());
    // A new profile no wider than Careful needs no acknowledgement and is not activated.
    let narrower = ProfileRows {
        read_event_details: Mode::Block,
        ..ProfileRows::CAREFUL
    };
    let epoch = run.counters().revocation_epoch;
    let answer = result(
        &save_profile(
            &mut run,
            0,
            custom("Team", Value::Null, narrower, Value::Null),
        )
        .0,
    );
    assert_eq!(answer["profiles"]["revision"], json!(1));
    assert_eq!(answer["profiles"]["active_profile_id"], json!("careful"));
    let profile_id = answer["profiles"]["items"][2]["profile_id"].clone();
    assert_eq!(run.counters().revocation_epoch, epoch);

    // A second version of a profile that is not active: the revision rises, the epoch does not.
    let answer = result(
        &save_profile(
            &mut run,
            1,
            custom("Team", profile_id.clone(), narrower, Value::Null),
        )
        .0,
    );
    assert_eq!(answer["profiles"]["revision"], json!(2));
    assert_eq!(answer["profiles"]["items"][2]["version"], json!(2));
    assert_eq!(run.counters().revocation_epoch, epoch);
    assert_eq!(active(&mut run), ("careful".to_owned(), 1));

    // Activated, then saved again: the new version is in force and the epoch rises each time.
    let _ = result(
        &save_profile(
            &mut run,
            2,
            json!({"kind": "activate", "profile_id": profile_id}),
        )
        .0,
    );
    assert_eq!(run.counters().revocation_epoch, epoch + 1);
    assert_eq!(
        active(&mut run),
        (profile_id.as_str().unwrap().to_owned(), 2)
    );
    // Widening a row of the active profile is measured against its previous version.
    let wider = ProfileRows {
        read_event_details: Mode::Ask,
        ..narrower
    };
    let refusal = error(
        &save_profile(
            &mut run,
            3,
            custom("Team", profile_id.clone(), wider, Value::Null),
        )
        .0,
    );
    assert_eq!(refusal["code"], json!("expansion_not_acknowledged"));
    assert_eq!(
        refusal["detail"]["expansion"],
        json!([{"row": "read_event_details", "from": "block", "to": "ask"}])
    );
    let digest = refusal["detail"]["expansion_digest"].clone();
    let answer = result(
        &save_profile(
            &mut run,
            3,
            custom("Team", profile_id.clone(), wider, digest),
        )
        .0,
    );
    assert_eq!(answer["profiles"]["revision"], json!(4));
    assert_eq!(answer["profiles"]["active_profile_id"], profile_id);
    assert_eq!(run.counters().revocation_epoch, epoch + 2);
    assert_eq!(
        active(&mut run),
        (profile_id.as_str().unwrap().to_owned(), 3)
    );

    let unknown = custom("Team", json!("custom-ffff"), narrower, Value::Null);
    assert_eq!(
        error(&save_profile(&mut run, 4, unknown).0)["code"],
        json!("not_found")
    );
}

#[test]
fn a_subscription_at_the_base_after_a_restart_is_accepted_and_below_it_is_not() {
    let mut run = Run::new(testing::isabelle());
    let _ = run.ok(
        "save_profile",
        json!({"expected_revision": 0, "change": {"kind": "activate", "profile_id": "flexible"}}),
    );
    run.restart();
    let cursor = run.snapshot()["cursor"].as_u64().unwrap();
    assert_eq!(cursor, 1);
    let below = run.refused("subscribe", json!({"cursor": 0}));
    assert_eq!(below["code"], json!("snapshot_required"));
    let ahead = run.refused("subscribe", json!({"cursor": 2}));
    assert_eq!(ahead["code"], json!("snapshot_required"));
    assert!(!run.broker.as_ref().unwrap().subscribed());
    let answer = run.ok("subscribe", json!({"cursor": 1}));
    assert_eq!(answer["cursor"], json!(1));
    assert!(run.broker.as_ref().unwrap().subscribed());
}

#[test]
fn get_proof_hides_a_synthetic_report() {
    let mut run = Run::new(synthetic());
    run.seed();
    let request_id = run.pending("call-1", "proof-synthetic");
    assert_eq!(run.details(&request_id)["proof_ids"], json!([]));
    let refusal = run.refused("get_proof", json!({"proof_id": "proof-synthetic"}));
    assert_eq!(refusal["code"], json!("not_found"));
    let refusal = run.refused("get_proof", json!({"proof_id": "proof-unknown"}));
    assert_eq!(refusal["code"], json!("not_found"));
    // The report is stored all the same.
    let stored = run.store(|s| s.read(|r| r.proof(&id("proof-synthetic"))).unwrap());
    assert_eq!(stored.map(|p| p.prover), Some(ProverKind::Synthetic));
}

#[test]
fn proof_ids_are_distinct_and_newest_first() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let output = run.approve(&request_id, "one_request");
    let fresh = jobs(&output).pop().expect("a fresh-permit job");
    let _ = run.prove(&fresh, "need_approval", "proof-2");
    assert_eq!(run.state(&request_id), "pending_approval");
    let output = run.approve(&request_id, "one_request");
    let fresh = jobs(&output).pop().expect("a fresh-permit job");
    let _ = run.prove(&fresh, "block", "proof-1");
    assert_eq!(run.state(&request_id), "blocked");
    assert_eq!(
        run.details(&request_id)["proof_ids"],
        json!(["proof-1", "proof-2"])
    );
    // The newest report with an id is the one shown.
    let report = run.ok("get_proof", json!({"proof_id": "proof-1"}));
    assert_eq!(report["outcome"], json!("block"));
    assert_eq!(report["purpose"], json!("fresh_permit"));
}

#[test]
fn full_details_carry_the_action_and_redacted_ones_do_not() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let redacted = run.details(&request_id);
    assert_eq!(redacted["private"], Value::Null);
    assert_eq!(redacted["redaction"], json!("redacted"));
    let full = run.ok(
        "get_request_details",
        json!({"request_id": request_id, "redaction": "full"}),
    );
    assert_eq!(
        full["private"]["action"]["tool"],
        json!("fabric_calendar_propose_create")
    );
    assert_eq!(full["private"]["before"], Value::Null);
    assert_eq!(full["private"]["warnings"], json!([]));
    assert!(full["card"].is_object());
    let refusal = run.refused(
        "get_request_details",
        json!({"request_id": "request-7", "redaction": "full"}),
    );
    assert_eq!(refusal["code"], json!("not_found"));
}

#[test]
fn disconnect_google_sends_the_account_notice_last_and_answers_with_its_sequence() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let approved = run.pending("call-1", "proof-1");
    let output = run.approve(&approved, "until_revoked");
    let grant_id = result(&response(&output))["approval"]["grant_id"].clone();
    let waiting = run.pending("call-2", "proof-2");
    // Reading the counters restarts the broker, so subscribe after it.
    let epoch = run.counters().revocation_epoch;
    let _ = run.subscribe();

    let (response, output) = run.owner(
        "disconnect_google",
        json!({"account_id": ACCOUNT, "expected_revision": 1, "revoke_provider_token": true}),
    );
    let answer = result(&response);
    let sent = notices_before_response(&output);
    let kinds: Vec<&str> = sent
        .iter()
        .map(|n| n["change"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["grant", "request", "account"]);
    assert_eq!(sent[0]["change"]["grant"]["grant_id"], grant_id);
    assert_eq!(
        sent[0]["change"]["grant"]["ended"]["reason"],
        json!("account_disconnected")
    );
    assert_eq!(sent[1]["change"]["request"]["request_id"], json!(waiting));
    assert_eq!(
        sent[1]["change"]["request"]["status"]["state"],
        json!("stale")
    );
    assert_eq!(answer["commit_sequence"], sent[2]["sequence"]);
    assert_eq!(answer["account"]["state"], json!("disconnected"));
    assert_eq!(answer["account"]["revision"], json!(2));
    assert_eq!(answer["ended_grant_ids"], json!([grant_id]));
    assert_eq!(run.counters().revocation_epoch, epoch + 1);

    let conflict = run.refused(
        "disconnect_google",
        json!({"account_id": ACCOUNT, "expected_revision": 1, "revoke_provider_token": false}),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], json!(2));
}

#[test]
fn pausing_all_agents_raises_each_revision_and_the_pause_epoch() {
    let mut run = Run::new(testing::isabelle());
    let refusal = run.refused(
        "pause_agent",
        json!({"target": {"kind": "all_agents"}, "paused": true}),
    );
    assert_eq!(refusal["code"], json!("not_found"));
    run.seed();
    // Reading the counters restarts the broker, which ends the session: read them first.
    let epoch = run.counters().pause_epoch;
    let before = run.snapshot()["agents"][0]["revision"].as_u64().unwrap();
    let answer = run.ok(
        "pause_agent",
        json!({"target": {"kind": "all_agents"}, "paused": true}),
    );
    assert_eq!(answer["agents"][0]["paused"], json!(true));
    assert_eq!(answer["agents"][0]["revision"], json!(before + 1));
    assert_eq!(run.counters().pause_epoch, epoch + 1);
    // A resume raises the revision again, and no epoch; so does a resume that changes nothing.
    for _ in 0..2 {
        let before = run.snapshot()["agents"][0]["revision"].as_u64().unwrap();
        let target = json!({"kind": "agent", "agent_id": AGENT, "expected_revision": before});
        let answer = run.ok("pause_agent", json!({"target": target, "paused": false}));
        assert_eq!(answer["agents"][0]["paused"], json!(false));
        assert_eq!(answer["agents"][0]["revision"], json!(before + 1));
    }
    assert_eq!(run.counters().pause_epoch, epoch + 1);
}

#[test]
fn a_revoked_grant_ends_and_a_second_revoke_is_conflict() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let request_id = run.pending("call-1", "proof-1");
    let output = run.approve(&request_id, "until_revoked");
    let grant_id = result(&response(&output))["approval"]["grant_id"].clone();
    let answer = run.ok(
        "revoke_grant",
        json!({"grant_id": grant_id, "expected_revision": 1}),
    );
    assert_eq!(
        answer["grant"]["ended"]["reason"],
        json!("revoked_by_owner")
    );
    assert_eq!(answer["grant"]["revision"], json!(2));
    let conflict = run.refused(
        "revoke_grant",
        json!({"grant_id": grant_id, "expected_revision": 2}),
    );
    assert_eq!(conflict["code"], json!("conflict"));
    assert_eq!(conflict["detail"]["current_revision"], json!(2));
    let unknown = run.refused(
        "revoke_grant",
        json!({"grant_id": "grant-7", "expected_revision": 1}),
    );
    assert_eq!(unknown["code"], json!("not_found"));
}

#[test]
fn a_warning_calendar_must_name_a_stored_calendar() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let settings = |calendar: &str| {
        json!({"zone": null, "workday": null, "protected_times": [], "uninterrupted": null,
               "buffer": null,
               "warning_calendars": [{"account_id": ACCOUNT, "calendar_id": calendar}]})
    };
    let refusal = run.refused(
        "save_preferences",
        json!({"expected_revision": 0, "settings": settings("cal-unknown")}),
    );
    assert_eq!(refusal["code"], json!("invalid_params"));
    assert_eq!(
        refusal["detail"]["pointer"],
        json!("/params/settings/warning_calendars/0")
    );
    let answer = run.ok(
        "save_preferences",
        json!({"expected_revision": 0, "settings": settings(CALENDAR)}),
    );
    assert_eq!(answer["preferences"]["revision"], json!(1));
}

/// c35. README section 7: `undecided_reason` is null unless the request is `undecided`, in full
/// and redacted answers alike, and an undecided request's redacted answer carries its reason. Each
/// answer decodes as a contract frame and has the members of the contract's fixture.
#[test]
fn details_carry_a_reason_only_when_undecided() {
    let mut run = Run::new(testing::isabelle());
    run.seed();
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/desktop/v1/fixtures/messages/",
            "accept-owner-response-get-request-details-undecided-failed.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let members = |result: &Value| -> Vec<String> {
        let mut keys: Vec<String> = result.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let expected = members(&fixture["value"]["result"]);
    let answer = |run: &mut Run, request_id: &str, redaction: &str| -> Value {
        let (_, output) = run.owner(
            "get_request_details",
            json!({"request_id": request_id, "redaction": redaction}),
        );
        let line = response_line(&output);
        let frame = decode(Entry::OwnerResponse, &line.as_bytes()[..line.len() - 1])
            .unwrap_or_else(|r| panic!("{r}: {line}"));
        let details = result(&frame.to_value());
        assert_eq!(members(&details), expected, "{request_id} {redaction}");
        details
    };

    let (blocked, output) = run.propose("call-1");
    let _ = run.prove(&jobs(&output).pop().unwrap(), "block", "proof-1");
    let pending = run.pending("call-2", "proof-2");
    let applied = run.pending("call-3", "proof-3");
    let output = run.approve(&applied, "until_revoked");
    let grant = result(&response(&output))["approval"]["grant_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let fresh = jobs(&output).pop().unwrap();
    let permit = ProofDone {
        authorizing_grants: vec![id(&grant)],
        ..done(&fresh, "permit", "proof-4")
    };
    let output = run.handle(Input::ProofDone(permit));
    let order = output
        .jobs
        .iter()
        .find_map(|j| match j {
            Job::Dispatch(order) => Some(order.clone()),
            Job::Prove(_) => None,
        })
        .expect("a dispatch order");
    let now = run.now;
    let _ = run.handle(Input::Attempt(AttemptDone {
        request_id: order.request_id,
        effect_id: order.effect_id,
        attempt: order.attempt,
        report: AttemptReport::Applied { event: None },
        finished_at_ms: now,
    }));
    let (failed, _) = run.propose_on("call-5", "cal-unknown");
    let (undecided, output) = run.propose("call-6");
    let _ = run.prove(&jobs(&output).pop().unwrap(), "undecided", "proof-6");

    let rows = [
        (&blocked, "blocked", "redacted"),
        (&pending, "pending_approval", "redacted"),
        (&pending, "pending_approval", "full"),
        (&applied, "applied", "redacted"),
        (&failed, "terminal_failure", "redacted"),
    ];
    for (request_id, state, redaction) in rows {
        let details = answer(&mut run, request_id, redaction);
        assert_eq!(details["request"]["status"]["state"], json!(state));
        assert_eq!(
            details["undecided_reason"],
            Value::Null,
            "{state} {redaction}"
        );
    }
    let details = answer(&mut run, &undecided, "redacted");
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_eq!(details["private"], Value::Null);
    assert_eq!(details["undecided_reason"], json!("failed"));
    let details = answer(&mut run, &undecided, "full");
    assert_eq!(details["request"]["status"]["state"], json!("undecided"));
    assert_ne!(details["private"], Value::Null, "{details}");
    assert_eq!(details["undecided_reason"], json!("failed"));
}
