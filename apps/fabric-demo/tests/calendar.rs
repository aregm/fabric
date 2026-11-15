//! The calendar service (tasks T4.1 and T4.2): token-gated free/busy and meetings, the owner
//! API, the encrypted store, and acceptance criterion A4, "no agent-facing response contains an
//! event title or event ID".

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use fabric_demo::audit::AuditLog;
use fabric_demo::calendar::{
    Calendar, CalendarClient, CalendarConfig, CalendarParts, CallError, Event, Store, StoreKey,
    TOKEN_SCHEME, bind, fixtures, free_busy, router, serve,
};
use fabric_demo::clock::{Clock, ManualClock};
use fabric_policy::time::{format_utc, parse_utc};
use fabric_policy::{Action, ActionVerdict, CheckKind, CheckReport, Verdict};
use fabric_token::{
    BindingKey, KeyDirectory, MemoryStore, Minter, PersonKeys, SecretStore, SigningKey, hex,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const OWNER_SECRET: &str = "owner-secret-for-tests-0123456789abcdef";
const PROOF_ID: &str = "5e1f0c2a9b8d7e6f5e1f0c2a9b8d7e6f5e1f0c2a9b8d7e6f5e1f0c2a9b8d7e6f";
const MINUTE: i64 = 60 * 1_000;
const HALF_HOUR: i64 = 30 * MINUTE;
const DAY: i64 = 24 * 60 * MINUTE;

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn scratch(test: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("calendar")
        .join(test);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the scratch directory can be created");
    directory
}

fn demo_fixtures(person: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../demo/fixtures")
        .join(format!("{person}.json"))
}

fn event(number: u32, start: &str, minutes: i64, busy: bool) -> Event {
    Event {
        id: format!("MARKER-ID-{number:02}"),
        start_ms: at(start),
        length_ms: minutes * MINUTE,
        title: format!("MARKER-TITLE-{number:02}"),
        busy,
        added_by: None,
    }
}

/// Fixtures whose titles and IDs are easy to search for.
fn marker_events() -> Vec<Event> {
    vec![
        event(1, "2027-01-11T07:00Z", 120, true), // straddles the window's start
        event(2, "2027-01-12T09:00Z", 60, true),
        event(3, "2027-01-12T10:00Z", 30, true), // touches event 2
        event(4, "2027-01-12T09:30Z", 15, true), // inside event 2
        event(5, "2027-01-12T13:00Z", 60, false),
        event(6, "2027-01-13T15:00Z", 60, true),
    ]
}

fn permit(action: &Action) -> CheckReport {
    CheckReport {
        kind: CheckKind::Authorize,
        theory: "Check_000009".to_owned(),
        proof_id: PROOF_ID.to_owned(),
        policy_sha256: "00".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: vec![ActionVerdict {
            action: action.clone(),
            verdict: Verdict::Permit,
            rules: Vec::new(),
        }],
        theory_text: String::new(),
    }
}

fn read(start: &str, end: &str) -> Action {
    Action::ReadFreeBusy {
        start_ms: at(start),
        end_ms: at(end),
    }
}

fn meeting(start: &str) -> Action {
    Action::AddMeeting {
        negotiation: "n1".to_owned(),
        start_ms: at(start),
        length_ms: HALF_HOUR,
    }
}

struct Reply {
    status: StatusCode,
    headers: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("JSON body: {}", self.body))
    }

    fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or_default().to_owned()
    }
}

/// Alice's calendar service with her gate and her agent, on a manual clock.
struct Harness {
    calendar: Arc<Calendar>,
    gate: Minter,
    agent: BindingKey,
    clock: Arc<ManualClock>,
    store_path: PathBuf,
    audit_path: PathBuf,
}

impl Harness {
    fn new(test: &str) -> Self {
        Self::with_events(test, marker_events())
    }

    fn with_events(test: &str, events: Vec<Event>) -> Self {
        let directory = scratch(test);
        let store_path = directory.join("data").join("alice.cal");
        fs::create_dir_all(store_path.parent().unwrap()).unwrap();
        let store =
            Store::open_or_create(&store_path, "alice", StoreKey::from_bytes([5; 32]), &events)
                .unwrap();
        let gate_key = SigningKey::from_bytes(&[7; 32]);
        let clock = Arc::new(ManualClock::new(at("2027-01-11T08:00Z")));
        let calendar = Calendar::new(CalendarParts {
            person: "alice".to_owned(),
            peer_name: "Bob".to_owned(),
            gate_key: gate_key.verifying_key(),
            owner_secret: OWNER_SECRET.to_owned(),
            store,
            fixtures: events,
            clock: clock.clone(),
            audit: AuditLog::open(&directory.join("audit.jsonl")).unwrap(),
        });
        Self {
            calendar,
            gate: Minter::new("alice", gate_key).unwrap(),
            agent: BindingKey::from_bytes(&[9; 32]),
            clock,
            store_path,
            audit_path: directory.join("audit.jsonl"),
        }
    }

    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    fn mint(&self, action: &Action) -> Vec<u8> {
        self.gate
            .mint(&permit(action), &self.agent.verifying_key(), self.now())
            .unwrap()
            .bytes
    }

    async fn send(&self, path: &str, token: &[u8], action: &Action) -> Reply {
        self.send_as(&self.agent, path, token, action).await
    }

    /// `agent` presents `token` with `action` as the body.
    async fn send_as(
        &self,
        agent: &BindingKey,
        path: &str,
        token: &[u8],
        action: &Action,
    ) -> Reply {
        let body = serde_json::to_vec(action).unwrap();
        let presentation = agent.present(token, &body, self.now()).unwrap();
        let authorization = format!(
            "{TOKEN_SCHEME} {}",
            hex::encode(&presentation.to_bytes().unwrap())
        );
        let request = Request::post(path)
            .header(AUTHORIZATION, authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        self.request(request).await
    }

    async fn request(&self, request: Request<Body>) -> Reply {
        let response = router(self.calendar.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let headers = format!("{:?}", response.headers());
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    async fn owner(&self, method: &str, path: &str, secret: Option<&str>) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(secret) = secret {
            request = request.header(AUTHORIZATION, format!("Bearer {secret}"));
        }
        self.request(request.body(Body::empty()).unwrap()).await
    }

    async fn owner_events(&self) -> Vec<Value> {
        let reply = self
            .owner("GET", "/v1/owner/events", Some(OWNER_SECRET))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.json()["events"].as_array().unwrap().clone()
    }
}

fn busy(start: &str, end: &str) -> Value {
    json!({ "start_ms": at(start), "end_ms": at(end) })
}

#[tokio::test]
async fn free_busy_shows_merged_busy_time_and_nothing_else() {
    let harness = Harness::new("free_busy");
    let window = read("2027-01-11T08:00Z", "2027-01-18T08:00Z");
    let reply = harness
        .send("/v1/freebusy", &harness.mint(&window), &window)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({ "busy": [
            busy("2027-01-11T08:00Z", "2027-01-11T09:00Z"),
            busy("2027-01-12T09:00Z", "2027-01-12T10:30Z"),
            busy("2027-01-13T15:00Z", "2027-01-13T16:00Z"),
        ]})
    );
}

#[tokio::test]
async fn add_meeting_writes_one_encrypted_event_to_this_calendar() {
    let harness = Harness::new("add_meeting");
    let action = meeting("2027-01-14T14:30Z");
    let reply = harness
        .send("/v1/meetings", &harness.mint(&action), &action)
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({ "start_ms": at("2027-01-14T14:30Z"), "length_ms": HALF_HOUR })
    );

    let events = harness.owner_events().await;
    let added: Vec<&Value> = events
        .iter()
        .filter(|event| event["title"] == "Meeting with Bob (Fabric)")
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["added_by"]["negotiation"], "n1");
    assert_eq!(added[0]["added_by"]["proof"], PROOF_ID);
    assert_eq!(added[0]["start_ms"], at("2027-01-14T14:30Z"));

    let window = read("2027-01-14T14:00Z", "2027-01-14T16:00Z");
    let reply = harness
        .send("/v1/freebusy", &harness.mint(&window), &window)
        .await;
    assert_eq!(
        reply.json(),
        json!({ "busy": [busy("2027-01-14T14:30Z", "2027-01-14T15:00Z")] })
    );

    // The meeting is on disk, and the file holds no title in the clear.
    let reopened = Store::open_or_create(
        &harness.store_path,
        "alice",
        StoreKey::from_bytes([5; 32]),
        &[],
    )
    .unwrap();
    assert_eq!(reopened.events().len(), 7);
    let bytes = fs::read(&harness.store_path).unwrap();
    for text in ["Meeting with Bob", "MARKER-TITLE", "MARKER-ID", "n1"] {
        assert!(
            !bytes
                .windows(text.len())
                .any(|window| window == text.as_bytes()),
            "{text} is readable in the store file"
        );
    }
}

/// Runs every kind of agent request against a service with `events`, and checks that no
/// response mentions an event's title or ID.
async fn check_no_titles_or_ids(harness: &Harness, events: &[Event]) {
    let mut replies = Vec::new();

    for (start, end) in [
        ("2027-01-11T08:00Z", "2027-01-18T08:00Z"),
        ("2027-01-12T09:10Z", "2027-01-12T09:20Z"),
        ("2027-01-16T00:00Z", "2027-01-17T00:00Z"),
    ] {
        let window = read(start, end);
        let reply = harness
            .send("/v1/freebusy", &harness.mint(&window), &window)
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        replies.push(reply);
    }

    let action = meeting("2027-01-14T14:30Z");
    let token = harness.mint(&action);
    let reply = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    replies.push(reply);
    replies.push(harness.send("/v1/meetings", &token, &action).await);
    replies.push(harness.send("/v1/freebusy", &token, &action).await);
    replies.push(
        harness
            .send(
                "/v1/meetings",
                &harness.mint(&action),
                &meeting("2027-01-14T14:15Z"),
            )
            .await,
    );
    let window = read("2027-01-11T08:00Z", "2027-01-18T08:00Z");
    let details = Action::ReadEventDetails {
        start_ms: at("2027-01-11T08:00Z"),
        end_ms: at("2027-01-18T08:00Z"),
    };
    replies.push(
        harness
            .send("/v1/freebusy", &harness.mint(&window), &details)
            .await,
    );
    let request = Request::post("/v1/freebusy")
        .body(Body::from("{\"kind\":\"read_free_busy\"}"))
        .unwrap();
    replies.push(harness.request(request).await);
    let request = Request::post("/v1/freebusy")
        .header(AUTHORIZATION, format!("{TOKEN_SCHEME} 00"))
        .body(Body::from(vec![b'x'; 20_000]))
        .unwrap();
    replies.push(harness.request(request).await);
    let request = Request::get("/v1/health").body(Body::empty()).unwrap();
    replies.push(harness.request(request).await);

    let statuses: Vec<u16> = replies.iter().map(|reply| reply.status.as_u16()).collect();
    assert_eq!(
        statuses,
        [200, 200, 200, 201, 403, 400, 403, 400, 401, 413, 200]
    );

    // The meeting the agent added is an event too.
    let mut markers: Vec<String> = events
        .iter()
        .flat_map(|event| [event.id.clone(), event.title.clone()])
        .collect();
    for event in harness.owner_events().await {
        markers.push(event["id"].as_str().unwrap().to_owned());
        markers.push(event["title"].as_str().unwrap().to_owned());
    }
    assert_eq!(markers.len(), 2 * events.len() + 2 * (events.len() + 1));
    for reply in &replies {
        for marker in &markers {
            assert!(
                !reply.body.contains(marker.as_str()) && !reply.headers.contains(marker.as_str()),
                "{marker:?} in {} {}",
                reply.status,
                reply.body
            );
        }
    }

    // Audit records hold no titles or event IDs either.
    // One record per agent request that reached the service (all but the oversized body and the
    // health check), and one for the owner's listing.
    let audit = fs::read_to_string(&harness.audit_path).unwrap();
    assert_eq!(audit.lines().count(), 9 + 1, "{audit}");
    for marker in &markers {
        assert!(
            !audit.contains(marker.as_str()),
            "{marker:?} in the audit log"
        );
    }
}

#[tokio::test]
async fn no_agent_response_carries_a_title_or_an_event_id() {
    let harness = Harness::new("no_titles_markers");
    check_no_titles_or_ids(&harness, &marker_events()).await;

    let events = fixtures::load(&demo_fixtures("alice"), "alice").unwrap();
    let harness = Harness::with_events("no_titles_demo", events.clone());
    check_no_titles_or_ids(&harness, &events).await;
}

#[tokio::test]
async fn misuse_is_refused_and_nothing_runs() {
    let harness = Harness::new("misuse");
    let action = meeting("2027-01-14T14:30Z");
    let token = harness.mint(&action);
    let window = read("2027-01-11T08:00Z", "2027-01-18T08:00Z");

    let request = Request::post("/v1/meetings")
        .body(Body::from(serde_json::to_vec(&action).unwrap()))
        .unwrap();
    let no_token = harness.request(request).await;

    let bob = Minter::new("bob", SigningKey::from_bytes(&[8; 32])).unwrap();
    let bobs_token = bob
        .mint(
            &permit(&action),
            &harness.agent.verifying_key(),
            harness.now(),
        )
        .unwrap()
        .bytes;
    let thief = BindingKey::from_bytes(&[10; 32]);
    let wider = read("2027-01-11T08:00Z", "2027-02-11T08:00Z");
    let details = Action::ReadEventDetails {
        start_ms: at("2027-01-11T08:00Z"),
        end_ms: at("2027-01-18T08:00Z"),
    };

    let cases = [
        (no_token, StatusCode::UNAUTHORIZED, "bad_signature"),
        (
            harness.send("/v1/freebusy", &token, &action).await,
            StatusCode::BAD_REQUEST,
            "wrong_endpoint",
        ),
        (
            harness
                .send("/v1/freebusy", &harness.mint(&window), &details)
                .await,
            StatusCode::BAD_REQUEST,
            "wrong_endpoint",
        ),
        // The over-broad read: a token for one window, a request for a wider one.
        (
            harness
                .send("/v1/freebusy", &harness.mint(&window), &wider)
                .await,
            StatusCode::FORBIDDEN,
            "token_argument_mismatch",
        ),
        // A token for a different slot.
        (
            harness
                .send("/v1/meetings", &token, &meeting("2027-01-14T14:15Z"))
                .await,
            StatusCode::FORBIDDEN,
            "token_argument_mismatch",
        ),
        (
            harness.send("/v1/meetings", &bobs_token, &action).await,
            StatusCode::UNAUTHORIZED,
            "bad_signature",
        ),
        (
            harness
                .send_as(&thief, "/v1/meetings", &token, &action)
                .await,
            StatusCode::UNAUTHORIZED,
            "pop_invalid",
        ),
    ];
    for (reply, status, error) in cases {
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (status, error),
            "{}",
            reply.body
        );
    }

    harness.clock.advance(61_000);
    let expired = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(
        (expired.status, expired.error().as_str()),
        (StatusCode::UNAUTHORIZED, "expired")
    );
    let added = |events: &[Value]| {
        events
            .iter()
            .filter(|event| event["title"] == "Meeting with Bob (Fabric)")
            .count()
    };
    assert_eq!(added(&harness.owner_events().await), 0, "nothing ran");

    let token = harness.mint(&action);
    let reply = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let replayed = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(
        (replayed.status, replayed.error().as_str()),
        (StatusCode::FORBIDDEN, "token_replayed")
    );
    assert_eq!(added(&harness.owner_events().await), 1);

    let request = Request::post("/v1/freebusy")
        .header(AUTHORIZATION, format!("{TOKEN_SCHEME} 00"))
        .body(Body::from(vec![b'x'; 20_000]))
        .unwrap();
    assert_eq!(
        harness.request(request).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn a_failed_write_releases_the_token() {
    let harness = Harness::new("failed_write");
    let action = meeting("2027-01-14T14:30Z");
    let token = harness.mint(&action);
    let data = harness.store_path.parent().unwrap().to_owned();

    fs::remove_dir_all(&data).unwrap();
    let failed = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(
        (failed.status, failed.error().as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, "store_failed")
    );

    fs::create_dir_all(&data).unwrap();
    let retried = harness.send("/v1/meetings", &token, &action).await;
    assert_eq!(retried.status, StatusCode::CREATED, "{}", retried.body);
    let titles: Vec<Value> = harness
        .owner_events()
        .await
        .into_iter()
        .map(|event| event["title"].clone())
        .filter(|title| title == "Meeting with Bob (Fabric)")
        .collect();
    assert_eq!(titles.len(), 1, "the failed write left nothing behind");
}

#[tokio::test]
async fn the_owner_api_needs_the_owner_secret() {
    let harness = Harness::new("owner");
    for secret in [None, Some("wrong"), Some(&OWNER_SECRET[1..])] {
        let reply = harness.owner("GET", "/v1/owner/events", secret).await;
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (StatusCode::UNAUTHORIZED, "owner_unauthorized")
        );
        assert!(!reply.body.contains("MARKER"));
    }
    let token = harness.mint(&read("2027-01-11T08:00Z", "2027-01-18T08:00Z"));
    let request = Request::get("/v1/owner/events")
        .header(
            AUTHORIZATION,
            format!("{TOKEN_SCHEME} {}", hex::encode(&token)),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        harness.request(request).await.status,
        StatusCode::UNAUTHORIZED
    );

    let events = harness.owner_events().await;
    assert_eq!(events.len(), 6);
    assert_eq!(events[0]["title"], "MARKER-TITLE-01");

    let action = meeting("2027-01-14T14:30Z");
    harness
        .send("/v1/meetings", &harness.mint(&action), &action)
        .await;
    assert_eq!(harness.owner_events().await.len(), 7);
    let refused = harness.owner("POST", "/v1/owner/reset", None).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    let reset = harness
        .owner("POST", "/v1/owner/reset", Some(OWNER_SECRET))
        .await;
    assert_eq!(reset.json(), json!({ "events": 6 }));
    assert_eq!(harness.owner_events().await.len(), 6);
}

#[test]
fn the_store_opens_only_with_its_key_and_person() {
    let directory = scratch("store");
    let path = directory.join("alice.cal");
    let key = || StoreKey::from_bytes([5; 32]);
    Store::open_or_create(&path, "alice", key(), &marker_events()).unwrap();
    let reopened = Store::open_or_create(&path, "alice", key(), &[]).unwrap();
    assert_eq!(reopened.events(), marker_events().as_slice());

    let unreadable = |path: &Path, person: &str, key: StoreKey| {
        let error = Store::open_or_create(path, person, key, &[]).unwrap_err();
        assert!(
            error.to_string().contains(path.to_str().unwrap()),
            "{error}"
        );
    };
    unreadable(&path, "alice", StoreKey::from_bytes([6; 32]));
    unreadable(&path, "bob", key());

    let bytes = fs::read(&path).unwrap();
    for (name, broken) in [
        ("flipped", {
            let mut flipped = bytes.clone();
            let last = flipped.len() - 1;
            flipped[last] ^= 1;
            flipped
        }),
        ("truncated", bytes[..20].to_vec()),
        ("no magic", bytes[8..].to_vec()),
    ] {
        let broken_path = directory.join(format!("{name}.cal"));
        fs::write(&broken_path, broken).unwrap();
        unreadable(&broken_path, "alice", key());
    }

    let secrets = MemoryStore::new();
    StoreKey::load_or_create(&secrets, "alice").unwrap();
    let stored = secrets.get("alice/store").unwrap().unwrap();
    assert_eq!(stored.len(), 32);
    let again = StoreKey::load_or_create(&secrets, "alice").unwrap();
    let path = directory.join("from-secrets.cal");
    Store::open_or_create(&path, "alice", again, &marker_events()).unwrap();
    let third = StoreKey::load_or_create(&secrets, "alice").unwrap();
    assert!(Store::open_or_create(&path, "alice", third, &[]).is_ok());
    secrets.put("alice/store", &[1; 16]).unwrap();
    assert!(StoreKey::load_or_create(&secrets, "alice").is_err());
}

#[test]
fn fixtures_are_checked() {
    let good = r#"{"format": "fabric-demo-fixtures-v1", "person": "alice", "events": [
        {"id": "a", "start": "2027-01-11T09:00Z", "minutes": 30, "title": "Standup", "busy": true}
    ]}"#;
    let events = fixtures::parse(good, "alice").unwrap();
    assert_eq!(events[0].length_ms, HALF_HOUR);
    assert!(fixtures::parse(good, "bob").is_err());

    let bad = [
        good.replace("fabric-demo-fixtures-v1", "fabric-demo-fixtures-v0"),
        good.replace("\"minutes\": 30", "\"minutes\": 0"),
        good.replace("2027-01-11T09:00Z", "11 January"),
        good.replace("\"Standup\"", "\" \""),
        good.replace("\"busy\": true}", "\"busy\": true, \"room\": \"4\"}"),
        good.replace(
            "\"busy\": true}",
            "\"busy\": true}, {\"id\": \"a\", \"start\": \"2027-01-11T10:00Z\", \"minutes\": 30, \
             \"title\": \"Again\", \"busy\": true}",
        ),
    ];
    for text in bad {
        assert!(fixtures::parse(&text, "alice").is_err(), "accepted {text}");
    }
}

/// The 30-minute slots on the 15-minute grid, in weekday working hours (09:00 to 17:00 UTC)
/// within the demo's 7-day window, that `events` leave free.
fn free_working_slots(events: &[Event]) -> Vec<String> {
    let (start, end) = (at("2027-01-11T08:00Z"), at("2027-01-18T08:00Z"));
    let busy = free_busy(events, start, end);
    let mut slots = Vec::new();
    let mut slot = start;
    while slot + HALF_HOUR <= end {
        let weekday = (slot.div_euclid(DAY) + 4) % 7; // 1970-01-01 was a Thursday; 0 is Sunday
        let minute = slot.rem_euclid(DAY) / MINUTE;
        let working = (1..=5).contains(&weekday) && minute >= 9 * 60 && minute + 30 <= 17 * 60;
        let clear = !busy
            .iter()
            .any(|interval| interval.start_ms < slot + HALF_HOUR && slot < interval.end_ms);
        if working && clear {
            slots.push(format_utc(slot));
        }
        slot += 15 * MINUTE;
    }
    slots
}

/// Section 14: Alice's free/busy suggests three afternoon candidates, Bob is busy for one, and
/// round 1 yields two mutual options. Gate G3 reviews the exact times.
#[test]
fn the_demo_fixtures_set_up_the_scenario() {
    let alice = fixtures::load(&demo_fixtures("alice"), "alice").unwrap();
    let bob = fixtures::load(&demo_fixtures("bob"), "bob").unwrap();
    let candidates = free_working_slots(&alice);
    assert_eq!(
        candidates,
        [
            "2027-01-12T14:00Z",
            "2027-01-13T15:00Z",
            "2027-01-14T14:30Z"
        ]
    );
    let bob_free = free_working_slots(&bob);
    let mutual: Vec<&String> = candidates
        .iter()
        .filter(|slot| bob_free.contains(slot))
        .collect();
    assert_eq!(mutual, ["2027-01-12T14:00Z", "2027-01-14T14:30Z"]);
}

#[test]
fn configs_are_checked_and_a_changed_store_key_fails_closed() {
    let directory = scratch("config");
    let secrets = MemoryStore::new();
    let mut keys = KeyDirectory::new();
    keys.insert(&PersonKeys::load_or_create(&secrets, "alice").unwrap());
    keys.write(&directory.join("keys.json")).unwrap();

    let config = json!({
        "person": "alice",
        "peer_name": "Bob",
        "listen": "127.0.0.1:0",
        "store": directory.join("data").join("alice.cal"),
        "fixtures": demo_fixtures("alice"),
        "keys": directory.join("keys.json"),
        "owner_secret": OWNER_SECRET,
        "clock": { "start": "2027-01-11T08:00:00Z", "wall_start_ms": 0 },
        "audit_log": directory.join("audit.jsonl"),
    });
    let write = |config: &Value| {
        let path = directory.join("calendar.json");
        fs::write(&path, serde_json::to_vec(config).unwrap()).unwrap();
        CalendarConfig::read(&path)
    };
    let parsed = write(&config).unwrap();
    assert_eq!(parsed.credential_prefix, "fabric-demo");

    let calendar = Calendar::from_config(&parsed, &secrets).unwrap();
    assert_eq!(calendar.component(), "calendar:alice");
    drop(calendar);
    Calendar::from_config(&parsed, &secrets).expect("the same key opens the store again");
    let error = Calendar::from_config(&parsed, &MemoryStore::new())
        .err()
        .expect("another key cannot open the store");
    assert!(error.contains("cannot decrypt"), "{error}");

    for (field, value) in [
        ("listen", json!("0.0.0.0:47001")),
        ("owner_secret", json!("short")),
        ("person", json!("alice smith")),
        ("extra", json!(true)),
    ] {
        let mut broken = config.clone();
        broken[field] = value;
        assert!(write(&broken).is_err(), "accepted {field}");
    }
}

#[tokio::test]
async fn the_service_answers_over_loopback() {
    let harness = Harness::new("loopback");
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(serve(
        listener,
        harness.calendar.clone(),
        std::future::pending(),
    ));
    let client = CalendarClient::new(&format!("http://{address}"));

    let window = read("2027-01-11T08:00Z", "2027-01-18T08:00Z");
    let token = harness.mint(&window);
    let busy = client
        .free_busy(&harness.agent, &token, &window, harness.now())
        .await
        .unwrap();
    assert_eq!(busy.len(), 3);
    let replayed = client
        .free_busy(&harness.agent, &token, &window, harness.now())
        .await
        .unwrap_err();
    assert!(
        matches!(&replayed, CallError::Refused { status: 403, error, .. } if error == "token_replayed"),
        "{replayed}"
    );

    let action = meeting("2027-01-14T14:30Z");
    client
        .add_meeting(
            &harness.agent,
            &harness.mint(&action),
            &action,
            harness.now(),
        )
        .await
        .unwrap();
    let events = client.owner_events(OWNER_SECRET).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.title == "Meeting with Bob (Fabric)")
    );
    assert!(matches!(
        client.owner_events("wrong").await,
        Err(CallError::Refused { status: 401, .. })
    ));
    assert_eq!(client.owner_reset(OWNER_SECRET).await.unwrap(), 6);

    assert!(bind("0.0.0.0:0".parse().unwrap()).await.is_err());
    server.abort();
}
