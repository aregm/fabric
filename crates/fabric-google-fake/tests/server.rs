//! f01, the server, the log and the faults: what the log holds, how faults fire, bounds on
//! what the server reads, and that it stops on drop.

mod common;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use common::*;
use fabric_google_fake::{FakeGoogle, Fault, FaultAction, LateTrigger, Match, SCOPE_EVENTS};
use serde_json::json;

fn timed(id: &str) -> serde_json::Value {
    json!({"id": id, "summary": id,
           "start": {"dateTime": "2026-11-18T10:00:00Z"}, "end": {"dateTime": "2026-11-18T11:00:00Z"}})
}

#[test]
fn the_log_holds_every_request_in_order_with_its_parts() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(ALICE_EMAIL, timed("logged1"));
    let etag = fake.with_state(|s| s.etag(ALICE_EMAIL, "logged1")).unwrap();
    api(
        &fake,
        &t,
        "GET",
        &format!(
            "{}?fields=items(id)&timeZone=Europe%2FBerlin",
            events_path(SHARED)
        ),
        &[],
        None,
    );
    api(
        &fake,
        &t,
        "PATCH",
        &format!("{}?sendUpdates=none", event_path(ALICE_EMAIL, "logged1")),
        &[("If-Match", &etag)],
        Some(&json!({"summary": "x"})),
    );
    form(
        &fake,
        "/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", "nope"),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ],
    );

    let log = fake.log();
    assert_eq!(log.len(), 3);
    let [read, patch, token_request] = log.entries() else {
        unreachable!()
    };
    assert_eq!(read.seq, 0);
    assert_eq!(read.method, "GET");
    assert_eq!(
        read.segments,
        vec!["calendar", "v3", "calendars", SHARED, "events"]
    );
    assert!(read.raw_path.contains("team%23ops%40group"));
    assert_eq!(read.query_value("fields"), Some("items(id)"));
    assert_eq!(read.query_value("timeZone"), Some("Europe/Berlin"));
    assert!(read.authorization);
    assert_eq!(read.token_account.as_deref(), Some(ALICE));
    assert_eq!(read.accept_encoding.as_deref(), Some("identity"));
    assert_eq!(read.answered, Some(200));

    assert_eq!(patch.if_match.as_deref(), Some(etag.as_str()));
    assert_eq!(patch.content_type.as_deref(), Some("application/json"));
    assert_eq!(patch.body_json(), Some(json!({"summary": "x"})));
    assert_eq!(patch.query_value("sendUpdates"), Some("none"));
    assert!(patch.is_calendar_change());

    assert!(!token_request.authorization);
    assert_eq!(
        token_request.form_value("grant_type"),
        Some("refresh_token")
    );
    assert_eq!(token_request.answered, Some(400));

    assert_eq!(log.calendar_changes().len(), 1);
    assert_eq!(log.method("GET").len(), 1);
    assert_eq!(log.path_prefix("/calendar/v3/calendars/").len(), 2);
    assert_eq!(log.token_requests().len(), 1);
    let connections: Vec<u64> = log.iter().map(|e| e.connection).collect();
    assert_eq!(connections, vec![1, 2, 3], "one request per connection");
    fake.clear_log();
    assert!(fake.log().is_empty());
}

#[test]
fn the_server_stops_and_frees_its_port_when_dropped() {
    let fake = FakeGoogle::start();
    let port = fake.port();
    let idle = fabric_google_fake::client::connect_idle(port).unwrap();
    let started = Instant::now();
    drop(fake);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "an idle connection does not hold the stop"
    );
    drop(idle);
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err());
    TcpListener::bind(addr).expect("the port is free again");
}

#[test]
fn dropping_the_fake_ends_a_delayed_answer() {
    let fake = fake();
    fake.arm(Fault::on(
        "GET",
        "/oauth2/v3/certs",
        FaultAction::Delay(Duration::from_secs(600)),
    ));
    let port = fake.port();
    let client = std::thread::spawn(move || request(port, "GET", "/oauth2/v3/certs", &[], b""));
    assert!(wait_until(|| fake.log().len() == 1), "the request arrived");
    let started = Instant::now();
    drop(fake);
    let answer = client.join().unwrap();
    assert!(!answer.complete, "no answer");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_delay_holds_the_answer_for_its_duration() {
    let fake = fake();
    fake.arm(Fault::on(
        "GET",
        "/oauth2/v3/certs",
        FaultAction::Delay(Duration::from_millis(300)),
    ));
    let started = Instant::now();
    let answer = request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"");
    assert_eq!(answer.status, 200);
    assert!(started.elapsed() >= Duration::from_millis(300));
}

#[test]
fn a_fault_fires_once_and_then_the_request_is_served_normally() {
    let fake = fake();
    let handle = fake.arm(Fault::on(
        "GET",
        "/oauth2/v3/certs",
        FaultAction::error(503, "backendError"),
    ));
    let other = fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::error(500, "backendError"),
    ));
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        503
    );
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        200
    );
    assert_eq!((handle.fired(), handle.remaining()), (1, 0));
    assert_eq!(other.fired(), 0, "it matches no request so far");
    assert_eq!(
        fake.log().entries()[0].faults,
        vec!["error 503 backendError"]
    );

    let twice = fake
        .arm(Fault::matching(Match::any(), FaultAction::error(429, "rateLimitExceeded")).times(2));
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        429
    );
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        429
    );
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        200
    );
    assert_eq!(twice.fired(), 2);
    fake.disarm_all();
    assert_eq!(
        api(
            &fake,
            &token(&fake, ALICE),
            "GET",
            "/users/me/calendarList",
            &[],
            None
        )
        .status,
        200
    );
}

#[test]
fn an_error_fault_answers_googles_shape_with_or_without_retry_after_and_applies_nothing() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars",
        FaultAction::error_retry_after(403, "rateLimitExceeded", "7"),
    ));
    let answer = api(
        &fake,
        &t,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&timed("ratelimited")),
    );
    assert_eq!(answer.status, 403);
    assert_eq!(answer.header("Retry-After"), Some("7"));
    let body = answer.json();
    assert_eq!(body["error"]["code"], 403);
    assert_eq!(body["error"]["errors"][0]["reason"], "rateLimitExceeded");
    assert_eq!(body["error"]["errors"][0]["domain"], "usageLimits");
    assert!(fake.events(ALICE_EMAIL).is_empty());

    fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars",
        FaultAction::error(409, "conflict"),
    ));
    let conflict = api(
        &fake,
        &t,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&timed("conflicted")),
    );
    assert_eq!(
        (conflict.status, conflict.header("Retry-After")),
        (409, None)
    );
}

#[test]
fn apply_then_close_applies_and_close_without_applying_does_not() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars",
        FaultAction::ApplyThenClose,
    ));
    let lost = api(
        &fake,
        &t,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&timed("applied1")),
    );
    assert!(!lost.complete);
    assert!(fake.event(ALICE_EMAIL, "applied1").is_some());
    assert_eq!(fake.log().last().unwrap().answered, None);

    fake.arm(Fault::on(
        "POST",
        "/calendar/v3/calendars",
        FaultAction::CloseWithoutApplying,
    ));
    let dropped = api(
        &fake,
        &t,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&timed("dropped1")),
    );
    assert!(!dropped.complete);
    assert!(fake.event(ALICE_EMAIL, "dropped1").is_none());
}

#[test]
fn apply_then_503_applies_the_change_and_answers_503() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(ALICE_EMAIL, timed("evt503"));
    fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars",
        FaultAction::apply_then_503(),
    ));
    let answer = api(
        &fake,
        &t,
        "PATCH",
        &event_path(ALICE_EMAIL, "evt503"),
        &[],
        Some(&json!({"summary": "new"})),
    );
    assert_eq!(
        (answer.status, answer.reason().as_deref()),
        (503, Some("backendError"))
    );
    assert_eq!(fake.event(ALICE_EMAIL, "evt503").unwrap()["summary"], "new");
}

#[test]
fn a_late_patch_lands_after_the_first_read_and_honours_its_if_match_then() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(ALICE_EMAIL, timed("latep"));
    let etag = fake.with_state(|s| s.etag(ALICE_EMAIL, "latep")).unwrap();
    fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/",
        FaultAction::ApplyLate(LateTrigger::AfterReads(1)),
    ));
    let path = event_path(ALICE_EMAIL, "latep");
    let sent = api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[("If-Match", &etag)],
        Some(&json!({"summary": "late"})),
    );
    assert!(!sent.complete, "no answer");
    let first = api(&fake, &t, "GET", &path, &[], None).json();
    assert_eq!(first["summary"], "latep");
    let second = api(&fake, &t, "GET", &path, &[], None).json();
    assert_eq!(second["summary"], "late");
    assert_eq!(fake.with_state(|s| s.late_changes_pending()), 0);

    // A late change whose event changed meanwhile fails its If-Match when it lands.
    let etag = fake.with_state(|s| s.etag(ALICE_EMAIL, "latep")).unwrap();
    fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/",
        FaultAction::ApplyLate(LateTrigger::Manual),
    ));
    api(
        &fake,
        &t,
        "PATCH",
        &path,
        &[("If-Match", &etag)],
        Some(&json!({"summary": "never"})),
    );
    fake.change_event(ALICE_EMAIL, "latep", json!({"location": "moved"}));
    assert_eq!(fake.apply_late_changes(), 1);
    assert_eq!(fake.event(ALICE_EMAIL, "latep").unwrap()["summary"], "late");
}

#[test]
fn body_faults_change_the_answer_bytes() {
    let fake = fake();
    let t = token(&fake, ALICE);
    let path = "/users/me/calendarList";
    fake.arm(Fault::on("GET", "/calendar/v3/", FaultAction::NotJson));
    let not_json = api(&fake, &t, "GET", path, &[], None);
    assert_eq!(not_json.status, 200);
    assert!(serde_json::from_slice::<serde_json::Value>(&not_json.body).is_err());

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::TooLong(5 * 1024 * 1024),
    ));
    let long = api(&fake, &t, "GET", path, &[], None);
    assert_eq!(long.body.len(), 5 * 1024 * 1024);
    assert!(long.json().is_object(), "still JSON, only long");

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::ContentType("text/plain".into()),
    ));
    assert_eq!(
        api(&fake, &t, "GET", path, &[], None).header("Content-Type"),
        Some("text/plain")
    );

    fake.arm(Fault::on("GET", "/calendar/v3/", FaultAction::Gzip));
    let gzip = api(&fake, &t, "GET", path, &[], None);
    assert_eq!(gzip.header("Content-Encoding"), Some("gzip"));
    assert_eq!(&gzip.body[..2], &[0x1f, 0x8b]);

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::Redirect("http://127.0.0.1:1/elsewhere".into()),
    ));
    let redirect = api(&fake, &t, "GET", path, &[], None);
    assert_eq!(
        (redirect.status, redirect.header("Location")),
        (302, Some("http://127.0.0.1:1/elsewhere"))
    );

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::Json(200, json!({"id": "other"})),
    ));
    assert_eq!(
        api(&fake, &t, "GET", path, &[], None).json(),
        json!({"id": "other"})
    );

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::Raw(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
    ));
    assert_eq!(api(&fake, &t, "GET", path, &[], None).body, b"{}");
}

#[test]
fn an_expired_token_fault_answers_401_and_the_token_stays_expired() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::ExpiredAccessToken,
    ));
    let path = "/users/me/calendarList";
    assert_eq!(api(&fake, &t, "GET", path, &[], None).status, 401);
    assert_eq!(api(&fake, &t, "GET", path, &[], None).status, 401);
    assert!(!fake.with_state(|s| s.access_token_valid(&t)));
}

#[test]
fn an_event_changed_by_someone_else_before_a_patch_makes_it_412() {
    let fake = fake();
    let t = token(&fake, ALICE);
    fake.put_event(ALICE_EMAIL, timed("raced"));
    let etag = fake.with_state(|s| s.etag(ALICE_EMAIL, "raced")).unwrap();
    fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/",
        FaultAction::change_event(ALICE_EMAIL, "raced", json!({"summary": "theirs"})),
    ));
    let answer = api(
        &fake,
        &t,
        "PATCH",
        &event_path(ALICE_EMAIL, "raced"),
        &[("If-Match", &etag)],
        Some(&json!({"summary": "mine"})),
    );
    assert_eq!(answer.status, 412);
    assert_eq!(
        fake.event(ALICE_EMAIL, "raced").unwrap()["summary"],
        "theirs"
    );

    fake.arm(Fault::on(
        "GET",
        "/calendar/v3/",
        FaultAction::before(|s| {
            s.set_access_role(ALICE, ALICE_EMAIL, "reader");
        }),
    ));
    api(&fake, &t, "GET", "/users/me/calendarList", &[], None);
    let write = api(
        &fake,
        &t,
        "DELETE",
        &event_path(ALICE_EMAIL, "raced"),
        &[],
        None,
    );
    assert_eq!(write.status, 403, "the role was lowered");
}

#[test]
fn a_panicking_hook_closes_its_connection_and_the_server_goes_on() {
    let fake = fake();
    fake.arm(Fault::on(
        "GET",
        "/oauth2/v3/certs",
        FaultAction::before(|_| panic!("a deliberate panic in a test hook")),
    ));
    let answer = request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"");
    assert!(!answer.complete);
    assert_eq!(fake.handler_panics(), 1);
    assert_eq!(
        fake.log().entries()[0].problem.as_deref(),
        Some("the fake's handler panicked")
    );
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        200
    );
    fake.with_state(|s| s.set_key_set_max_age(1));
}

#[test]
fn requests_the_server_cannot_read_are_logged_and_answered_4xx() {
    let fake = fake();
    let port = fake.port();
    let host = format!("Host: 127.0.0.1:{port}\r\n");
    let cases: Vec<(Vec<u8>, u16)> = vec![
        (
            format!("GET / HTTP/1.1\r\n{host}X: {}\r\n\r\n", "a".repeat(20_000)).into_bytes(),
            431,
        ),
        (format!("GET / HTTP/1.1\n{host}\n").into_bytes(), 400),
        (
            format!("POST /token HTTP/1.1\r\n{host}Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n")
                .into_bytes(),
            411,
        ),
        (
            format!(
                "POST /token HTTP/1.1\r\n{host}Content-Length: 1\r\nContent-Length: 2\r\n\r\nab"
            )
            .into_bytes(),
            400,
        ),
        (
            format!("POST /token HTTP/1.1\r\n{host}Content-Length: 2000000\r\n\r\n").into_bytes(),
            413,
        ),
        (b"GET / HTTP/1.1\r\n\r\n".to_vec(), 400),
        (format!("GET / HTTP/1.0\r\n{host}\r\n").into_bytes(), 505),
        (
            format!("GET /a%zz HTTP/1.1\r\n{host}\r\n").into_bytes(),
            400,
        ),
        (format!("get / HTTP/1.1\r\n{host}\r\n").into_bytes(), 400),
    ];
    for (bytes, status) in &cases {
        let answer = send_bytes(port, bytes);
        assert_eq!(
            answer.status,
            *status,
            "{}",
            String::from_utf8_lossy(&bytes[..40.min(bytes.len())])
        );
    }
    let log = fake.log();
    assert_eq!(log.len(), cases.len());
    assert!(log.iter().all(|e| e.problem.is_some()));
    assert_eq!(request(port, "GET", "/nowhere", &[], b"").status, 404);
    assert_eq!(request(port, "GET", "/token", &[], b"").status, 405);
}

#[test]
fn a_body_sent_in_pieces_is_read_to_its_content_length() {
    let fake = fake();
    let t = token(&fake, ALICE);
    let body = timed("pieces1").to_string();
    let head = format!(
        "POST /calendar/v3/calendars/{}/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {t}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        enc(ALICE_EMAIL),
        fake.port(),
        body.len()
    );
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, fake.port()));
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(BOUND)).unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    stream.flush().unwrap();
    let (a, b) = body.as_bytes().split_at(10);
    stream.write_all(a).unwrap();
    stream.flush().unwrap();
    assert!(wait_until(|| fake.connections() == 1));
    stream.write_all(b).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    assert!(answer.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(fake.event(ALICE_EMAIL, "pieces1").is_some());
}

#[test]
fn idle_connections_are_counted_but_not_logged() {
    let fake = fake();
    let idle: Vec<TcpStream> = (0..3)
        .map(|_| fabric_google_fake::client::connect_idle(fake.port()).unwrap())
        .collect();
    assert!(wait_until(|| fake.connections() == 3));
    assert_eq!(
        request(fake.port(), "GET", "/oauth2/v3/certs", &[], b"").status,
        200
    );
    assert_eq!(fake.log().len(), 1);
    assert_eq!(fake.log().entries()[0].connection, 4);
    drop(idle);
}

#[test]
fn every_request_of_a_flow_comes_on_its_own_connection_and_writes_need_the_events_scope() {
    let fake = fake();
    let read_only = fake.issue_access_token(
        ALICE,
        CLIENT_ID,
        &["https://www.googleapis.com/auth/calendar.events.readonly"],
        3600,
    );
    let refused = api(
        &fake,
        &read_only,
        "POST",
        &events_path(ALICE_EMAIL),
        &[],
        Some(&timed("noscope")),
    );
    assert_eq!(refused.reason().as_deref(), Some("insufficientPermissions"));
    let writer = fake.issue_access_token(ALICE, CLIENT_ID, &[SCOPE_EVENTS], 3600);
    assert_eq!(
        api(
            &fake,
            &writer,
            "POST",
            &events_path(ALICE_EMAIL),
            &[],
            Some(&timed("scoped01"))
        )
        .status,
        200
    );
    let log = fake.log();
    let mut connections: Vec<u64> = log.iter().map(|e| e.connection).collect();
    connections.dedup();
    assert_eq!(connections.len(), log.len());
}
