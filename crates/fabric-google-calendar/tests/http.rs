//! The rules of `http` (5.2) against a scripted peer on `127.0.0.1`, without the fake Google
//! server: the three results (g38), redirects, bounds, content types and compression (g39),
//! one connection per request, the headers sent, no proxy, and no leak in any formatted value
//! (in the spirit of g16).

#![cfg(debug_assertions)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{ClosedPort, Reply, ScriptedPeer, Witness, answer, answer_until_close};
use fabric_google_calendar::http::{
    BodyBound, BodyError, Exchange, Method, NotSentReason, Request, Target, Transport, USER_AGENT,
    UnansweredReason,
};
use fabric_google_calendar::{
    CalendarId, Clock, Endpoints, EventId, ManualClock, Revision, Secret, SystemClock,
};

/// A deadline far enough away for the peer's answers, short enough to fail fast.
const SOON_MS: u64 = 5_000;

fn transport(port: u16) -> Transport {
    Transport::new(Endpoints::loopback(port), Arc::new(SystemClock)).expect("a transport")
}

fn deadline(ms: u64) -> u64 {
    SystemClock.now_ms() + ms
}

fn get_list() -> Request {
    Request::new(Method::Get, Target::CalendarList, BodyBound::Large)
}

fn json_ok(body: &[u8]) -> Vec<u8> {
    answer(
        "200 OK",
        &[("Content-Type", "application/json; charset=UTF-8")],
        body,
    )
}

fn expect_answer(exchange: Exchange) -> fabric_google_calendar::http::Answer {
    match exchange {
        Exchange::Answer(answer) => answer,
        other => panic!("expected an answer, got {other:?}"),
    }
}

fn expect_unanswered(exchange: &Exchange) -> UnansweredReason {
    match exchange {
        Exchange::SentUnanswered(reason) => *reason,
        other => panic!("expected sent and unanswered, got {other:?}"),
    }
}

#[test]
fn a_closed_port_is_nothing_sent() {
    let closed = ClosedPort::new();
    let t = transport(closed.port());
    let result = t.send(get_list(), deadline(SOON_MS));
    assert!(
        matches!(result, Exchange::NothingSent(NotSentReason::ConnectFailed)),
        "{result:?}"
    );
}

#[test]
fn a_deadline_already_past_is_nothing_sent_and_makes_no_connection() {
    let witness = Witness::start();
    let clock = ManualClock::new(1_000_000);
    let t = Transport::new(Endpoints::loopback(witness.port()), Arc::new(clock.clone()))
        .expect("a transport");
    for past in [0, 999_999, 1_000_000] {
        let result = t.send(get_list(), past);
        assert!(
            matches!(result, Exchange::NothingSent(NotSentReason::DeadlinePassed)),
            "{result:?}"
        );
    }
    assert!(!witness.was_contacted());
}

#[test]
fn a_peer_that_accepts_and_closes_is_sent_and_unanswered() {
    let peer = ScriptedPeer::start(vec![Reply::AcceptAndClose]);
    let result = transport(peer.port()).send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&result), UnansweredReason::Transport);
    peer.finish();
}

#[test]
fn a_peer_that_reads_the_request_and_closes_is_sent_and_unanswered() {
    let peer = ScriptedPeer::start(vec![Reply::ReadAndClose]);
    let result = transport(peer.port()).send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&result), UnansweredReason::Transport);
    let connections = peer.finish();
    assert_eq!(connections[0].requests.len(), 1, "the request was sent");
}

#[test]
fn a_peer_that_stalls_past_the_deadline_is_sent_and_unanswered_at_the_deadline() {
    let peer = ScriptedPeer::start(vec![Reply::Stall]);
    let started = Instant::now();
    let result = transport(peer.port()).send(get_list(), deadline(300));
    let took = started.elapsed();
    assert_eq!(expect_unanswered(&result), UnansweredReason::TimedOut);
    assert!(took >= Duration::from_millis(250), "{took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
    let connections = peer.finish();
    assert_eq!(connections[0].requests.len(), 1);
}

#[test]
fn a_body_that_stalls_past_the_deadline_is_sent_and_unanswered() {
    // The head and part of the body arrive, then nothing: the deadline covers the body too.
    let partial =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10\r\n\r\n{\"a\""
            .to_vec();
    let peer = ScriptedPeer::start(vec![Reply::Answer(partial)]);
    let started = Instant::now();
    let result = transport(peer.port()).send(get_list(), deadline(300));
    assert_eq!(expect_unanswered(&result), UnansweredReason::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    peer.finish();
}

#[test]
fn half_an_answer_is_sent_and_unanswered() {
    let half =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"items\":"
            .to_vec();
    let peer = ScriptedPeer::start(vec![
        Reply::AnswerAndClose(half),
        Reply::AnswerAndClose(b"HTTP/1.1 200 OK\r\nContent-Ty".to_vec()),
    ]);
    let t = transport(peer.port());
    let body_cut = t.send(get_list(), deadline(SOON_MS));
    assert_eq!(
        expect_unanswered(&body_cut),
        UnansweredReason::BodyIncomplete
    );
    let head_cut = t.send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&head_cut), UnansweredReason::Transport);
    peer.finish();
}

#[test]
fn a_redirect_is_returned_as_an_answer_and_not_followed() {
    let elsewhere = Witness::start();
    let location = format!(
        "http://127.0.0.1:{}/calendar/v3/users/me/calendarList",
        elsewhere.port()
    );
    for status in [
        "301 Moved Permanently",
        "302 Found",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let peer = ScriptedPeer::start(vec![Reply::Answer(answer(
            status,
            &[("Location", &location)],
            b"",
        ))]);
        let a = expect_answer(transport(peer.port()).send(get_list(), deadline(SOON_MS)));
        assert_eq!(a.status(), status[..3].parse::<u16>().expect("a number"));
        assert!(!a.is_success());
        peer.finish();
    }
    assert!(!elsewhere.was_contacted());
}

#[test]
fn each_body_bound_holds_exactly_its_size_and_refuses_one_byte_more() {
    for bound in [BodyBound::Small, BodyBound::Medium, BodyBound::Large] {
        let size = bound.bytes();
        let fits = vec![b' '; size];
        let over = vec![b' '; size + 1];
        let peer = ScriptedPeer::start(vec![
            Reply::Answer(json_ok(&fits)),
            Reply::Answer(json_ok(&over)),
            Reply::AnswerAndClose(answer_until_close(
                "200 OK",
                &[("Content-Type", "application/json")],
                &fits,
            )),
            Reply::AnswerAndClose(answer_until_close(
                "200 OK",
                &[("Content-Type", "application/json")],
                &over,
            )),
        ]);
        let t = transport(peer.port());
        let request = || Request::new(Method::Get, Target::Keys, bound);
        let a = expect_answer(t.send(request(), deadline(SOON_MS)));
        assert_eq!(a.body().len(), size);
        let r = t.send(request(), deadline(SOON_MS));
        assert_eq!(
            expect_unanswered(&r),
            UnansweredReason::BodyTooLarge,
            "{bound:?}"
        );
        let a = expect_answer(t.send(request(), deadline(SOON_MS)));
        assert_eq!(a.body().len(), size, "{bound:?} without a length");
        let r = t.send(request(), deadline(SOON_MS));
        assert_eq!(
            expect_unanswered(&r),
            UnansweredReason::BodyTooLarge,
            "{bound:?} without a length"
        );
        peer.finish();
    }
}

#[test]
fn an_error_answer_is_held_to_64_kib_whatever_the_bound() {
    let fits = vec![b' '; 64 * 1024];
    let over = vec![b' '; 64 * 1024 + 1];
    let headers = [("Content-Type", "application/json")];
    let peer = ScriptedPeer::start(vec![
        Reply::Answer(answer("404 Not Found", &headers, &fits)),
        Reply::Answer(answer("503 Service Unavailable", &headers, &over)),
    ]);
    let t = transport(peer.port());
    let a = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert_eq!(a.status(), 404);
    let r = t.send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&r), UnansweredReason::BodyTooLarge);
    peer.finish();
}

#[test]
fn a_json_answer_must_say_application_json() {
    let body = br#"{"kind":"calendar#calendarList","items":[]}"#;
    let peer = ScriptedPeer::start(vec![
        Reply::Answer(answer("200 OK", &[("Content-Type", "text/html")], body)),
        Reply::Answer(answer("200 OK", &[], body)),
        Reply::Answer(json_ok(body)),
        Reply::Answer(json_ok(b"<html>")),
    ]);
    let t = transport(peer.port());
    let wrong = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert_eq!(wrong.status(), 200);
    assert_eq!(wrong.json().err(), Some(BodyError::NotJson));
    let missing = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert_eq!(missing.json().err(), Some(BodyError::NotJson));
    let right = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert_eq!(right.json().expect("JSON")["kind"], "calendar#calendarList");
    let broken = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert_eq!(broken.json().err(), Some(BodyError::Unreadable));
    peer.finish();
}

#[test]
fn a_compressed_answer_is_no_usable_answer() {
    let headers_gzip = [
        ("Content-Type", "application/json"),
        ("Content-Encoding", "gzip"),
    ];
    let headers_identity = [
        ("Content-Type", "application/json"),
        ("Content-Encoding", "identity"),
    ];
    let gzip_bytes = [0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
    let peer = ScriptedPeer::start(vec![
        Reply::Answer(answer("200 OK", &headers_gzip, &gzip_bytes)),
        Reply::Answer(answer("200 OK", &[("Content-Encoding", "br")], b"x")),
        Reply::Answer(answer("200 OK", &headers_identity, b"{}")),
    ]);
    let t = transport(peer.port());
    let r = t.send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&r), UnansweredReason::Compressed);
    let r = t.send(get_list(), deadline(SOON_MS));
    assert_eq!(expect_unanswered(&r), UnansweredReason::Compressed);
    let a = expect_answer(t.send(get_list(), deadline(SOON_MS)));
    assert!(a.json().is_ok());
    peer.finish();
}

#[test]
fn retry_after_and_cache_control_are_read_from_the_answer() {
    let peer = ScriptedPeer::start(vec![Reply::Answer(answer(
        "429 Too Many Requests",
        &[
            ("Content-Type", "application/json"),
            ("Retry-After", "17"),
            ("Cache-Control", "public, max-age=21600"),
        ],
        b"{}",
    ))]);
    let a = expect_answer(transport(peer.port()).send(get_list(), deadline(SOON_MS)));
    assert_eq!(a.status(), 429);
    assert_eq!(a.retry_after_seconds(), Some(17));
    assert_eq!(a.max_age_seconds(), Some(21_600));
    peer.finish();
}

#[test]
fn every_request_opens_its_own_connection_and_none_is_reused() {
    // The peer answers without `Connection: close` and keeps reading, so a client that kept
    // the connection would send its next request on it.
    let ok = json_ok(b"{}");
    let peer = ScriptedPeer::start(vec![
        Reply::Answer(ok.clone()),
        Reply::Answer(ok.clone()),
        Reply::Answer(ok),
    ]);
    let t = transport(peer.port());
    for _ in 0..3 {
        expect_answer(t.send(get_list(), deadline(SOON_MS)));
    }
    let connections = peer.finish();
    assert_eq!(connections.len(), 3);
    for connection in &connections {
        assert_eq!(connection.requests.len(), 1, "one request per connection");
        assert_eq!(
            connection.requests[0].header("connection").as_deref(),
            Some("close")
        );
    }
}

#[test]
fn the_crate_sends_its_own_headers_and_no_others() {
    let ok = json_ok(b"{}");
    let peer = ScriptedPeer::start(vec![
        Reply::Answer(ok.clone()),
        Reply::Answer(ok.clone()),
        Reply::Answer(ok.clone()),
        Reply::Answer(ok),
    ]);
    let t = transport(peer.port());
    let calendar = CalendarId::new("team#x@group.calendar.google.com").expect("valid");
    let event = EventId::new("abc_123").expect("valid");
    let revision = Revision::new("\"3381\"").expect("valid");
    let token = Secret::from("ya29.token-value");

    let get = Request::new(
        Method::Get,
        Target::Events(calendar.clone()),
        BodyBound::Large,
    )
    .query("timeZone", "Europe/Berlin")
    .query("pageToken", "p&q=r")
    .bearer(&token)
    .expect("a usable token");
    expect_answer(t.send(get, deadline(SOON_MS)));

    let patch = Request::new(
        Method::Patch,
        Target::Event(calendar.clone(), event.clone()),
        BodyBound::Medium,
    )
    .query("sendUpdates", "none")
    .bearer(&token)
    .expect("a usable token")
    .if_match(&revision)
    .expect("a header")
    .json(&serde_json::json!({"summary": "Lunch"}))
    .expect("a body");
    expect_answer(t.send(patch, deadline(SOON_MS)));

    let form = Request::new(Method::Post, Target::Token, BodyBound::Small)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", "1//r t&x=y"),
        ])
        .expect("a form for the token endpoint");
    expect_answer(t.send(form, deadline(SOON_MS)));

    let delete = Request::new(
        Method::Delete,
        Target::Event(calendar, event),
        BodyBound::Small,
    )
    .query("sendUpdates", "all")
    .bearer(&token)
    .expect("a usable token")
    .if_match(&revision)
    .expect("a header");
    expect_answer(t.send(delete, deadline(SOON_MS)));

    let connections = peer.finish();
    let requests: Vec<_> = connections.iter().map(|c| c.requests[0].clone()).collect();
    let common_names = [
        "accept",
        "accept-encoding",
        "connection",
        "user-agent",
        "host",
    ];
    let names = |extra: &[&str]| {
        let mut all: Vec<String> = common_names
            .iter()
            .chain(extra)
            .map(|s| (*s).to_owned())
            .collect();
        all.sort();
        all
    };
    let sorted = |r: &common::Captured| {
        let mut n = r.header_names();
        n.sort();
        n
    };
    for r in &requests {
        assert_eq!(r.header("user-agent").as_deref(), Some(USER_AGENT));
        assert_eq!(r.header("accept").as_deref(), Some("application/json"));
        assert_eq!(r.header("accept-encoding").as_deref(), Some("identity"));
        assert_eq!(r.header("connection").as_deref(), Some("close"));
        assert!(
            r.request_line().ends_with(" HTTP/1.1"),
            "{}",
            r.request_line()
        );
    }

    let get = &requests[0];
    assert_eq!(
        get.request_line(),
        "GET /calendar/v3/calendars/team%23x%40group.calendar.google.com/events?timeZone=Europe%2FBerlin&pageToken=p%26q%3Dr HTTP/1.1"
    );
    assert_eq!(sorted(get), names(&["authorization"]));
    assert_eq!(
        get.header("authorization").as_deref(),
        Some("Bearer ya29.token-value")
    );

    let patch = &requests[1];
    assert_eq!(
        patch.request_line(),
        "PATCH /calendar/v3/calendars/team%23x%40group.calendar.google.com/events/abc_123?sendUpdates=none HTTP/1.1"
    );
    assert_eq!(
        sorted(patch),
        names(&[
            "authorization",
            "if-match",
            "content-type",
            "content-length"
        ])
    );
    assert_eq!(patch.header("if-match").as_deref(), Some("\"3381\""));
    assert_eq!(
        patch.header("content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(patch.body, br#"{"summary":"Lunch"}"#);

    let form = &requests[2];
    assert_eq!(form.request_line(), "POST /token HTTP/1.1");
    assert_eq!(sorted(form), names(&["content-type", "content-length"]));
    assert_eq!(
        form.header("content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        form.body,
        b"grant_type=refresh_token&refresh_token=1%2F%2Fr%20t%26x%3Dy"
    );

    let delete = &requests[3];
    assert_eq!(
        delete.request_line(),
        "DELETE /calendar/v3/calendars/team%23x%40group.calendar.google.com/events/abc_123?sendUpdates=all HTTP/1.1"
    );
    assert_eq!(sorted(delete), names(&["authorization", "if-match"]));
}

#[test]
fn a_google_transport_needs_tls_and_has_it_only_on_macos_and_windows() {
    // No request is made: building the client contacts nobody.
    let result = Transport::new(Endpoints::google(), Arc::new(SystemClock));
    if cfg!(any(windows, target_os = "macos")) {
        assert!(result.is_ok());
    } else {
        assert_eq!(
            result.err(),
            Some(fabric_google_calendar::http::SetupError::TlsUnavailable)
        );
    }
}

/// The marker that turns [`proxy_variables_child`] from a no-op into a real check:
/// `transport:<port>` or `control:<port>`.
const PROXY_CHILD: &str = "FABRIC_GC_PROXY_CHILD";

/// Runs [`proxy_variables_child`] in a child process of this test binary, with every proxy
/// variable pointing at `proxy`. Returns the child's standard output.
fn run_proxy_child(mode: &str, proxy: &Witness) -> String {
    let proxy_url = format!("http://127.0.0.1:{}", proxy.port());
    let exe = std::env::current_exe().expect("the test binary");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "proxy_variables_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PROXY_CHILD, mode)
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("all_proxy", &proxy_url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .output()
        .expect("run the child");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "the child failed: {stdout}");
    assert!(stdout.contains("1 passed"), "the child ran: {stdout}");
    stdout
}

/// The proxy variables of the environment are ignored (5.2: no proxy, whatever the
/// environment says). Setting variables in this process would race with other tests (and
/// `set_var` is `unsafe` in edition 2024, which the workspace forbids), so the check runs in
/// child processes of this test binary with the variables set only there. A control child
/// shows that a reqwest client without the crate's settings does take the variables.
#[test]
fn the_proxy_variables_of_the_environment_are_ignored() {
    let control_proxy = Witness::start();
    let closed = ClosedPort::new();
    let unused = closed.port();
    let stdout = run_proxy_child(&format!("control:{unused}"), &control_proxy);
    assert!(stdout.contains("proxy child: control done"), "{stdout}");
    assert!(
        control_proxy.was_contacted(),
        "the control shows the variables are in effect"
    );

    let proxy = Witness::start();
    let peer = ScriptedPeer::start(vec![Reply::Answer(json_ok(b"{}"))]);
    let stdout = run_proxy_child(&format!("transport:{}", peer.port()), &proxy);
    assert!(stdout.contains("proxy child: direct"), "{stdout}");
    let connections = peer.finish();
    assert_eq!(
        connections[0].requests.len(),
        1,
        "the peer was reached directly"
    );
    assert!(!proxy.was_contacted(), "the proxy was never contacted");
}

/// The child half of the proxy check. Without [`PROXY_CHILD`] it does nothing.
#[test]
fn proxy_variables_child() {
    let Ok(mode) = std::env::var(PROXY_CHILD) else {
        return;
    };
    let (mode, port) = mode.split_once(':').expect("mode:port");
    let port: u16 = port.parse().expect("a port");
    let url = format!("http://127.0.0.1:{port}/calendar/v3/users/me/calendarList");
    if mode == "control" {
        // reqwest's defaults read the proxy variables: this request goes to the proxy, which
        // never answers, and ends at the time-out.
        let control = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(300))
            .build()
            .expect("a control client");
        let _ = control.get(url).send();
        println!("proxy child: control done");
    } else {
        let a = expect_answer(transport(port).send(get_list(), deadline(SOON_MS)));
        assert_eq!(a.status(), 200);
        println!("proxy child: direct");
    }
}

#[test]
fn no_formatted_result_or_error_holds_an_address_a_header_value_a_body_or_a_secret() {
    const TOKEN: &str = "ya29.SENTINEL-TOKEN";
    const CAL: &str = "sentinel-cal@group.calendar.google.com";
    const QUERY: &str = "SENTINEL-QUERY";
    const BODY: &str = "SENTINEL-BODY";
    const HEADER: &str = "SENTINEL-HEADER";
    let leaks = |text: &str, port: u16| {
        let port = port.to_string();
        for needle in [
            TOKEN,
            CAL,
            "sentinel",
            "SENTINEL",
            QUERY,
            BODY,
            HEADER,
            "127.0.0.1",
            "http:",
            "/calendar",
            &port,
        ] {
            assert!(!text.contains(needle), "{needle} leaked in {text}");
        }
    };
    let request = || {
        Request::new(
            Method::Patch,
            Target::Event(
                CalendarId::new(CAL).expect("valid"),
                EventId::new("SENTINEL_EVENT").expect("valid"),
            ),
            BodyBound::Small,
        )
        .query("q", QUERY)
        .bearer(&Secret::from(TOKEN))
        .expect("a usable token")
        .if_match(&Revision::new("\"SENTINEL-REV\"").expect("valid"))
        .expect("a header")
        .json(&serde_json::json!({ "summary": BODY }))
        .expect("a body")
    };
    let with_body = |status: &str, body: &[u8]| {
        Reply::Answer(answer(
            status,
            &[
                ("Content-Type", "text/plain"),
                ("X-Echo", HEADER),
                ("Location", HEADER),
            ],
            body,
        ))
    };
    let peer = ScriptedPeer::start(vec![
        Reply::AcceptAndClose,
        Reply::ReadAndClose,
        Reply::Stall,
        Reply::AnswerAndClose(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nSENTINEL".to_vec()),
        with_body("200 OK", &vec![b'S'; 70_000]),
        with_body("302 Found", BODY.as_bytes()),
        with_body("200 OK", BODY.as_bytes()),
        Reply::AnswerAndClose(b"HTTP/1.1 SENTINEL\r\n\r\n".to_vec()),
    ]);
    let port = peer.port();
    let t = transport(port);
    let mut texts = vec![format!("{t:?} {:?}", request())];
    for (i, ms) in [
        SOON_MS, SOON_MS, 300, SOON_MS, SOON_MS, SOON_MS, SOON_MS, SOON_MS,
    ]
    .into_iter()
    .enumerate()
    {
        let result = t.send(request(), deadline(ms));
        texts.push(format!("{i}: {result:?} {result:#?}"));
        match &result {
            Exchange::NothingSent(r) => texts.push(r.to_string()),
            Exchange::SentUnanswered(r) => texts.push(r.to_string()),
            Exchange::Answer(a) => {
                if let Err(e) = a.json() {
                    texts.push(format!("{e} {e:?}"));
                }
            }
        }
    }
    peer.finish();
    // A connect error and a deadline already past.
    let closed_port = ClosedPort::new();
    let closed = closed_port.port();
    let t = transport(closed);
    let r = t.send(request(), deadline(SOON_MS));
    texts.push(format!("{r:?}"));
    leaks(&format!("{r:?}"), closed);
    let r = t.send(request(), 0);
    texts.push(format!("{r:?}"));
    // Request errors.
    let e = Request::new(Method::Get, Target::CalendarList, BodyBound::Small)
        .bearer(&Secret::from("SENTINEL TOKEN"))
        .expect_err("refused");
    texts.push(format!("{e} {e:?}"));
    for text in &texts {
        leaks(text, port);
    }
}
