//! The loopback listener against the fake's browser (design 3.3): g07, g08, g09.

#![cfg(debug_assertions)]

mod oauth_common;

use std::io::{ErrorKind, Read};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use fabric_google_calendar::oauth::listener::{ERROR_TEXT, PAGE_TEXT, POLL};
use fabric_google_calendar::oauth::{CancelReason, ConnectJob, ConnectOutcome};
use fabric_google_fake::client::{RawRequest, connect_idle};
use fabric_google_fake::{ClientError, Decision, HttpResponse, Tamper};
use oauth_common::*;

/// A guard for joins: far longer than any wait the listener has.
const GUARD: Duration = Duration::from_secs(20);

/// A callback sent after the flow's listener closed finds no listener: the connection is
/// refused, or, when another test's listener has since been given the same port, what answers
/// is not this flow's page.
fn assert_no_listener(result: Result<HttpResponse, ClientError>) {
    match result {
        Err(ClientError::Refused) => {}
        Ok(answer) => assert!(
            !(answer.status == 200 && answer.text().contains(PAGE_TEXT)),
            "a closed flow's listener answered the callback"
        ),
        Err(other) => panic!("neither refused nor answered: {other:?}"),
    }
}

/// Whether the server has not closed `stream`: a read that would block finds it open.
fn still_open(stream: &mut TcpStream) -> bool {
    stream.set_nonblocking(true).expect("non-blocking");
    let mut byte = [0u8; 1];
    let open = matches!(stream.read(&mut byte), Err(e) if e.kind() == ErrorKind::WouldBlock);
    stream.set_nonblocking(false).expect("blocking");
    open
}

#[test]
fn g07_wrong_requests_get_error_pages_and_never_end_the_flow_and_the_right_state_ends_it_once() {
    let rig = Rig::new();
    let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
    let port = pending.port().expect("listening");
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        // `wait` opens the browser, so the authorization runs once it has.
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        let authorization = rig
            .fake
            .browser()
            .authorize(address.expose(), &Decision::allow())
            .expect("authorized");
        assert_eq!(authorization.status, 302);
        let location = authorization.location.expect("a redirect");
        let browser = rig.fake.browser();
        let wrong: Vec<(Tamper, u16)> = vec![
            (Tamper::State("wrong-state".to_owned()), 400),
            (Tamper::DropState, 400),
            (Tamper::DoubleParameter("state".to_owned()), 400),
            (Tamper::Path("/other".to_owned()), 404),
            (Tamper::Path("/favicon.ico".to_owned()), 404),
            (Tamper::Method("POST".to_owned()), 400),
            (Tamper::Method("HEAD".to_owned()), 400),
            (Tamper::Host(format!("localhost:{port}")), 400),
            (Tamper::Host("rebound.example".to_owned()), 400),
        ];
        for i in 0..60 {
            let (tamper, status) = &wrong[i % wrong.len()];
            let answer = browser
                .follow(&location, std::slice::from_ref(tamper))
                .expect("the listener answers");
            assert_eq!(answer.status, *status, "{tamper:?}");
            if !matches!(tamper, Tamper::Method(m) if m == "HEAD") {
                assert!(answer.text().contains(ERROR_TEXT), "{tamper:?}");
            }
        }
        assert!(
            !waiter.is_finished(),
            "sixty wrong requests did not end the flow"
        );
        // Unknown parameters are ignored.
        let right = browser
            .follow(
                &location,
                &[
                    Tamper::AddParameter("hd".to_owned(), "example.com".to_owned()),
                    Tamper::AddParameter("error_description".to_owned(), "x".to_owned()),
                    Tamper::AddParameter("unknown".to_owned(), "y".to_owned()),
                ],
            )
            .expect("the listener answers");
        assert_eq!(right.status, 200);
        assert!(right.text().contains(PAGE_TEXT));
        let outcome = waiter.join().expect("the flow ends");
        assert!(outcome.is_connected(), "{outcome:?}");
        // A second callback finds no listener.
        assert_no_listener(browser.follow(&location, &[]));
    });
    assert_eq!(rig.token_requests("authorization_code"), 1);
    rig.check_log();
}

#[test]
fn g07_a_wrong_iss_a_doubled_code_or_both_code_and_error_with_the_right_state_is_google_error() {
    let rig = Rig::new();
    for tamper in [
        Tamper::Iss("https://evil.example".to_owned()),
        Tamper::DoubleParameter("iss".to_owned()),
        Tamper::DoubleParameter("code".to_owned()),
        Tamper::AddParameter("error".to_owned(), "access_denied".to_owned()),
        Tamper::DropParameter("code".to_owned()),
    ] {
        let label = format!("{tamper:?}");
        let (outcome, visit, _) = rig.sign_in(
            ConnectJob::new_account(account(1), []),
            Decision::allow().tamper(tamper),
        );
        assert_eq!(
            outcome,
            ConnectOutcome::Cancelled(CancelReason::GoogleError),
            "{label}"
        );
        assert_eq!(visit.listener_answer().expect("answered").status, 200);
    }
    // Without `iss` the callback is accepted (it is checked only when present).
    let (outcome, _, _) = rig.sign_in(
        ConnectJob::new_account(account(1), []),
        Decision::allow().tamper(Tamper::DropParameter("iss".to_owned())),
    );
    assert!(outcome.is_connected(), "{outcome:?}");
    assert_eq!(rig.token_requests("authorization_code"), 1);
    assert_eq!(rig.store.writes(), 1);
    rig.check_log();
}

fn join_within<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    wait_until(GUARD, || handle.is_finished());
    handle.join().expect("the flow ends")
}

#[test]
fn g08_the_listener_ends_after_300_seconds_on_the_test_clock_with_timed_out() {
    let rig = Rig::new();
    rig.connect(1, Decision::allow());
    let (pending, address) = rig.start(ConnectJob::reconnect(account(1), SUB_A));
    let port = pending.port().expect("listening");
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        rig.clock.advance(299_999);
        // Still alive one millisecond before the end: it answers.
        let probe = RawRequest::get(&format!("http://127.0.0.1:{port}/?state=x"))
            .expect("a request")
            .send()
            .expect("answered");
        assert_eq!(probe.status, 400);
        assert!(!waiter.is_finished());
        rig.clock.advance(1);
        assert_eq!(
            join_within(waiter),
            ConnectOutcome::Cancelled(CancelReason::TimedOut)
        );
    });
    // The listener is closed, and a reconnect that timed out forgets the old token (3.7).
    let late = rig
        .fake
        .browser()
        .visit(address.expose(), Decision::allow())
        .expect("visit");
    assert_no_listener(late.callback.expect("a redirect"));
    assert!(rig.store.token(&account(1)).is_none());
    assert!(rig.fake.log().revocations().is_empty());
    rig.check_log();
}

#[test]
fn g08_a_superseded_flow_ends_at_once_with_superseded() {
    let rig = Rig::new();
    let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
    let handle = pending.supersede_handle();
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        // Superseded once `wait` has opened the browser, so the listener runs.
        let _ = address.expose();
        let started = Instant::now();
        handle.supersede();
        assert_eq!(
            join_within(waiter),
            ConnectOutcome::Cancelled(CancelReason::Superseded)
        );
        // 3.3 asks for 250 ms: the accept loop reads the flag every `POLL`. The end-to-end
        // bound is generous for a loaded machine.
        assert!(POLL <= Duration::from_millis(250), "{POLL:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
    });
    let late = rig
        .fake
        .browser()
        .visit(address.expose(), Decision::allow())
        .expect("visit");
    assert_no_listener(late.callback.expect("a redirect"));
    assert!(rig.store.inner.is_empty());
    // A flow superseded before it waits ends the same way.
    let (pending, _) = rig.start(ConnectJob::new_account(account(2), []));
    pending.supersede_handle().supersede();
    let mut store = rig.store.clone();
    assert_eq!(
        pending.wait(&mut store, &rig.cache),
        ConnectOutcome::Cancelled(CancelReason::Superseded)
    );
    rig.check_log();
}

/// 3.2.1: `wait` opens the browser; one that cannot be started ends the flow
/// `BrowserNotOpened`, its listener closed and no request made.
#[test]
fn g08_a_browser_that_cannot_be_started_ends_the_flow_browser_not_opened() {
    let rig = Rig::new();
    let mut browser = fabric_google_calendar::oauth::RecordingBrowser::refusing();
    let pending = rig.flow.start(ConnectJob::new_account(account(1), []));
    let port = pending.port().expect("listening until `wait` runs");
    let mut store = rig.store.clone();
    assert_eq!(
        pending.wait(&mut browser, &mut store, &rig.cache),
        ConnectOutcome::Cancelled(CancelReason::BrowserNotOpened)
    );
    assert_no_listener(
        RawRequest::get(&format!("http://127.0.0.1:{port}/?state=x"))
            .expect("a request")
            .send(),
    );
    assert!(rig.fake.log().is_empty());
}

#[test]
fn g09_the_page_is_the_fixed_text_with_the_three_headers_and_echoes_no_parameter() {
    let rig = Rig::new();
    let (outcome, visit, address) =
        rig.sign_in(ConnectJob::new_account(account(1), []), Decision::allow());
    assert!(outcome.is_connected());
    let page = visit.listener_answer().expect("answered");
    assert_eq!(page.status, 200);
    assert_eq!(page.header("Cache-Control"), Some("no-store"));
    assert_eq!(
        page.header("Content-Type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(page.header("Connection"), Some("close"));
    let text = page.text();
    assert!(text.contains(PAGE_TEXT));
    let code = visit.code().expect("a code");
    for echoed in [
        code.as_str(),
        &query_value(address.expose(), "state").expect("state"),
        "accounts.google.com",
        "authuser",
        "scope",
    ] {
        assert!(!text.contains(echoed), "{echoed}");
    }

    // An error page echoes nothing either.
    let (pending, address) = rig.start(ConnectJob::new_account(account(2), []));
    let handle = pending.supersede_handle();
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        let location = rig
            .fake
            .browser()
            .authorize(address.expose(), &Decision::allow())
            .expect("authorized")
            .location
            .expect("a redirect");
        let answer = rig
            .fake
            .browser()
            .follow(&location, &[Tamper::State("echo-me-<b>".to_owned())])
            .expect("answered");
        assert_eq!(answer.status, 400);
        assert!(!answer.text().contains("echo-me"));
        assert_eq!(answer.header("Cache-Control"), Some("no-store"));
        handle.supersede();
        join_within(waiter);
    });
    rig.check_log();
}

#[test]
fn g09_seven_idle_connections_do_not_delay_the_callback() {
    let rig = Rig::new();
    let (pending, address) = rig.start(ConnectJob::new_account(account(1), []));
    let port = pending.port().expect("listening");
    let mut idle: Vec<_> = (0..7)
        .map(|_| connect_idle(port).expect("an idle connection"))
        .collect();
    let mut store = rig.store.clone();
    let cache = &rig.cache;
    std::thread::scope(|scope| {
        let waiter = scope.spawn(move || pending.wait(&mut store, cache));
        let visit = rig
            .fake
            .browser()
            .visit(address.expose(), Decision::allow())
            .expect("visit");
        assert_eq!(visit.listener_answer().expect("answered").status, 200);
        // The callback was answered while every idle connection was still open: it did not
        // wait for a slot that one of them held.
        for (i, stream) in idle.iter_mut().enumerate() {
            assert!(still_open(stream), "idle connection {i} was closed first");
        }
        assert!(join_within(waiter).is_connected());
    });
    drop(idle);
    rig.check_log();
}
