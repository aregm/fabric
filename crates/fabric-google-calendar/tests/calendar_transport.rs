//! Classification against the fake, the transport's three cases, the safety rules of every
//! changing request, the id checks, and what formatted values may hold: g37 to g42, and the
//! Calendar half of g16.

mod calendar_common;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use calendar_common::{
    Person, World, ZONE, create, delete, event_ref, invitation, no_changes, put, query, reserved,
    response, timed, update,
};
use fabric_desktop_contracts::agent::{Action, ProposedResponse};
use fabric_desktop_contracts::common::{EventField, FailureCode, SendUpdates};
use fabric_desktop_contracts::scalar::EventTitle;
use fabric_google_calendar::calendar::{CalendarTarget, ChangeOrder, Google, Observed};
use fabric_google_calendar::http::Transport;
use fabric_google_calendar::outcome::{ChangeOutcome, ChangeReport, ReadFailure, SideResult};
use fabric_google_calendar::reconcile::Effect;
use fabric_google_calendar::{AccessFailure, AuthLostCause, Clock, Endpoints, TokenStoreError};
use fabric_google_fake::{ASKED_SCOPES, CalendarEntry, Fault, FaultAction, LateTrigger, Match};
use serde_json::json;

const TITLE: &str = "Secret Board Meeting";

fn meeting(world: &World, target: &CalendarTarget, id: &str) -> (String, String) {
    put(
        world,
        target,
        json!({
            "id": id, "summary": TITLE,
            "start": {"dateTime": "2026-11-20T09:00:00+01:00", "timeZone": ZONE},
            "end": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": ZONE},
            "organizer": {"email": "carol@example.com"},
            "attendees": [
                {"email": "alice@example.com", "self": true, "responseStatus": "needsAction"},
                {"email": "carol@example.com", "organizer": true, "responseStatus": "accepted"},
            ],
        }),
    )
}

fn retitle(target: &CalendarTarget, id: &str, etag: &str) -> Action {
    let mut c = no_changes();
    c.title = Some(EventTitle::new("Renamed").expect("valid"));
    update(target, event_ref(id, etag), c, SendUpdates::All)
}

fn run(
    world: &mut World,
    who: &Person,
    target: &CalendarTarget,
    action: Action,
    id: Option<String>,
    deadline: u64,
) -> ChangeReport {
    let order = ChangeOrder::new(action, id, 1);
    world.as_account(who, |g| g.change(&who.id, target, &order, deadline))
}

fn code(report: &ChangeReport) -> Option<FailureCode> {
    match &report.outcome {
        ChangeOutcome::Rejected { code, .. } => Some(*code),
        _ => None,
    }
}

fn read_code(failure: &ReadFailure) -> Option<FailureCode> {
    match failure {
        ReadFailure::Rejected { code, .. } => Some(*code),
        _ => None,
    }
}

fn renew_token(world: &World, who: &Person) {
    let token =
        world
            .fake
            .issue_access_token(&who.subject, calendar_common::CLIENT, &ASKED_SCOPES, 3600);
    world.tokens.set(&who.id, token);
}

/// g37: one case per row of 6.10, for a change (an update) and for a read (an observation and
/// an events list); `Retry-After` is passed on and capped.
#[test]
fn each_row_of_6_10_classifies_a_change_and_a_read_against_the_fake() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();
    type Want = (
        Option<ChangeOutcome>,
        Option<FailureCode>,
        Option<SideResult>,
        ReadWant,
    );
    #[derive(Debug, PartialEq)]
    enum ReadWant {
        Transient(Option<u64>),
        Rejected(FailureCode),
    }
    let t = |ms| Some(ChangeOutcome::Transient { retry_after_ms: ms });
    let cases: Vec<(&str, FaultAction, Want)> = vec![
        (
            "400",
            FaultAction::error(400, "invalid"),
            (
                None,
                Some(FailureCode::InvalidArguments),
                None,
                ReadWant::Rejected(FailureCode::InvalidArguments),
            ),
        ),
        (
            "401",
            FaultAction::ExpiredAccessToken,
            (t(None), None, None, ReadWant::Transient(None)),
        ),
        (
            "403 rate",
            FaultAction::error_retry_after(403, "rateLimitExceeded", "30"),
            (
                t(Some(30_000)),
                None,
                None,
                ReadWant::Transient(Some(30_000)),
            ),
        ),
        (
            "429 user rate",
            FaultAction::error_retry_after(429, "userRateLimitExceeded", "500"),
            (
                t(Some(120_000)),
                None,
                None,
                ReadWant::Transient(Some(120_000)),
            ),
        ),
        (
            "429 no hint",
            FaultAction::error(429, "rateLimitExceeded"),
            (t(None), None, None, ReadWant::Transient(None)),
        ),
        (
            "403 quota",
            FaultAction::error(403, "quotaExceeded"),
            (
                None,
                Some(FailureCode::ProviderRejected),
                None,
                ReadWant::Rejected(FailureCode::ProviderRejected),
            ),
        ),
        (
            "403 organizer",
            FaultAction::error(403, "forbiddenForNonOrganizer"),
            (
                None,
                Some(FailureCode::ProviderRejected),
                None,
                ReadWant::Rejected(FailureCode::ProviderRejected),
            ),
        ),
        (
            "403 scope",
            FaultAction::error(403, "insufficientPermissions"),
            (
                None,
                Some(FailureCode::AccountAuthenticationRequired),
                Some(SideResult::AuthLost(AuthLostCause::GrantEnded)),
                ReadWant::Rejected(FailureCode::AccountAuthenticationRequired),
            ),
        ),
        (
            "403 other",
            FaultAction::error(403, "forbidden"),
            (
                None,
                Some(FailureCode::ProviderRejected),
                Some(SideResult::AccessChanged),
                ReadWant::Rejected(FailureCode::ProviderRejected),
            ),
        ),
        (
            "500",
            FaultAction::error(500, "backendError"),
            (
                Some(ChangeOutcome::NoAnswer),
                None,
                None,
                ReadWant::Transient(None),
            ),
        ),
        (
            "503",
            FaultAction::error(503, "backendError"),
            (
                Some(ChangeOutcome::NoAnswer),
                None,
                None,
                ReadWant::Transient(None),
            ),
        ),
        (
            "302",
            FaultAction::Redirect("http://127.0.0.1:9/elsewhere".into()),
            (
                None,
                Some(FailureCode::ProviderRejected),
                None,
                ReadWant::Rejected(FailureCode::ProviderRejected),
            ),
        ),
        (
            "418",
            FaultAction::error(418, "teapot"),
            (
                None,
                Some(FailureCode::ProviderRejected),
                None,
                ReadWant::Rejected(FailureCode::ProviderRejected),
            ),
        ),
    ];
    for (label, action, (outcome, rejected, side, read)) in cases {
        let (id, etag) = meeting(&world, &p, "meeting1");
        world.fake.arm(Fault::on(
            "PATCH",
            "/calendar/v3/calendars/",
            action.clone(),
        ));
        let report = run(
            &mut world,
            &alice,
            &p,
            retitle(&p, &id, &etag),
            None,
            deadline,
        );
        if let Some(outcome) = outcome {
            assert_eq!(report.outcome, outcome, "{label}");
        }
        assert_eq!(code(&report), rejected, "{label}");
        assert_eq!(report.side, side, "{label}");
        renew_token(&world, &alice);
        for kind in ["event", "list"] {
            world
                .fake
                .arm(Fault::on("GET", "/calendar/v3/calendars/", action.clone()));
            let failure = if kind == "event" {
                world
                    .as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline))
                    .expect_err(label)
            } else {
                let q = query(std::slice::from_ref(&p), &[EventField::Time]);
                world
                    .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
                    .expect_err(label)
            };
            let got = match &failure {
                ReadFailure::Transient { retry_after_ms } => ReadWant::Transient(*retry_after_ms),
                ReadFailure::Rejected { code, .. } => ReadWant::Rejected(*code),
                ReadFailure::Stopped => panic!("{label}: stopped"),
            };
            assert_eq!(got, read, "{label} {kind}");
            assert_eq!(failure.side(), side, "{label} {kind}");
            renew_token(&world, &alice);
        }
    }
    // A 401 dropped the cached token, once per call that got it.
    assert_eq!(world.tokens.dropped().len(), 3);
    // 404 on an event is Absent; 404 on a calendar is Rejected.
    assert_eq!(
        world.as_account(&alice, |g| g
            .observe(&alice.id, &p, "nosuch", false, deadline)),
        Ok(Observed::Absent)
    );
    let mut gone = p.clone();
    gone.provider_id =
        fabric_google_calendar::CalendarId::new("gone@group.example.com").expect("valid");
    let q = query(&[gone], &[EventField::Time]);
    let failure = world
        .as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
        .expect_err("404");
    assert_eq!(read_code(&failure), Some(FailureCode::ProviderRejected));
    world.check_log();
}

/// g37: the refresh before the call ended `AuthLost`, or the token store failed: no request,
/// and the classes of 3.8 and section 4.
#[test]
fn a_call_without_a_token_makes_no_request_and_follows_3_8() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p, "meeting1");
    let deadline = world.deadline();
    let cases = [
        (AccessFailure::Transient, None, None),
        (AccessFailure::Store(TokenStoreError::Locked), None, None),
        (
            AccessFailure::Store(TokenStoreError::Refused),
            Some(FailureCode::AccountAuthenticationRequired),
            None,
        ),
        (
            AccessFailure::Store(TokenStoreError::Failed { status: -25_300 }),
            Some(FailureCode::AccountAuthenticationRequired),
            None,
        ),
        (
            AccessFailure::AuthLost(AuthLostCause::GrantEnded),
            Some(FailureCode::AccountAuthenticationRequired),
            Some(SideResult::AuthLost(AuthLostCause::GrantEnded)),
        ),
        (
            AccessFailure::AuthLost(AuthLostCause::ClientRejected),
            Some(FailureCode::AccountAuthenticationRequired),
            Some(SideResult::AuthLost(AuthLostCause::ClientRejected)),
        ),
    ];
    for (failure, want, side) in cases {
        world.tokens.fail_with(Some(failure));
        let report = run(
            &mut world,
            &alice,
            &p,
            retitle(&p, &id, &etag),
            None,
            deadline,
        );
        assert_eq!(code(&report), want, "{failure:?}");
        assert_eq!(report.side, side);
        if want.is_none() {
            assert_eq!(
                report.outcome,
                ChangeOutcome::Transient {
                    retry_after_ms: None
                }
            );
        }
        let read = world
            .as_account(&alice, |g| g.list_calendars(&alice.id, deadline))
            .expect_err("no token");
        assert_eq!(read_code(&read), want);
        assert_eq!(read.side(), side);
    }
    world.tokens.fail_with(None);
    assert!(world.fake.log().is_empty(), "no request without a token");
    world.check_log();
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("an address").port()
}

/// g38: nothing sent (nothing listening, the deadline already past, fewer than 10 s left for a
/// change) is Transient; sent and unanswered is NoAnswer for a change and Transient for a read;
/// every request came on its own connection.
#[test]
fn nothing_sent_is_transient_and_sent_unanswered_is_no_answer_for_a_change_only() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let (id, etag) = meeting(&world, &p, "meeting1");
    let now = world.clock.now_ms();

    // Nothing listening.
    let transport = Transport::new(
        Endpoints::loopback(closed_port()),
        Arc::new(world.clock.clone()),
    )
    .expect("a transport");
    let mut closed = Google::new(
        Arc::new(transport),
        Box::new(world.tokens.clone()),
        Arc::new(world.clock.clone()),
    );
    let order = ChangeOrder::new(retitle(&p, &id, &etag), None, 1);
    assert_eq!(
        closed.change(&alice.id, &p, &order, now + 30_000).outcome,
        ChangeOutcome::transient()
    );
    assert_eq!(
        closed.observe(&alice.id, &p, &id, false, now + 30_000),
        Err(ReadFailure::transient())
    );

    // The deadline already past, and fewer than 10 s left for a change: no request.
    for deadline in [now - 1, now, now + 9_999] {
        let report = run(
            &mut world,
            &alice,
            &p,
            retitle(&p, &id, &etag),
            None,
            deadline,
        );
        assert_eq!(report.outcome, ChangeOutcome::transient());
    }
    for deadline in [now - 1, now] {
        let read = world.as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline));
        assert_eq!(read, Err(ReadFailure::transient()));
    }
    assert!(world.fake.log().is_empty());
    // A read with 9 s left is sent.
    assert!(
        world
            .as_account(&alice, |g| g.observe(
                &alice.id,
                &p,
                &id,
                false,
                now + 9_000
            ))
            .is_ok()
    );

    // Sent and unanswered.
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::CloseWithoutApplying,
    ));
    let report = run(
        &mut world,
        &alice,
        &p,
        retitle(&p, &id, &etag),
        None,
        now + 30_000,
    );
    assert_eq!(report.outcome, ChangeOutcome::NoAnswer);
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::CloseWithoutApplying,
    ));
    let read = world.as_account(&alice, |g| {
        g.observe(&alice.id, &p, &id, false, now + 30_000)
    });
    assert_eq!(read, Err(ReadFailure::transient()));
    // An answer later than the deadline: a read with 300 ms of deadline and a 3 s delay.
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::Delay(Duration::from_secs(3)),
    ));
    let read = world.as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, now + 300));
    assert_eq!(read, Err(ReadFailure::transient()));

    let log = world.fake.log();
    let connections: std::collections::HashSet<u64> = log.iter().map(|e| e.connection).collect();
    assert_eq!(
        connections.len(),
        log.len(),
        "every request on its own connection"
    );
    world.check_log();
}

/// g39: a redirect is not followed; a body over its bound, a compressed body, a wrong content
/// type and a body that is not JSON are no usable answer.
#[test]
fn a_redirect_is_not_followed_and_an_unusable_body_is_no_answer() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();
    let witness = TcpListener::bind("127.0.0.1:0").expect("bind");
    witness.set_nonblocking(true).expect("non-blocking");
    let elsewhere = format!(
        "http://127.0.0.1:{}/calendar/v3/elsewhere",
        witness.local_addr().expect("addr").port()
    );
    let (id, etag) = meeting(&world, &p, "meeting1");
    world.fake.arm(Fault::on(
        "PATCH",
        "/calendar/v3/calendars/",
        FaultAction::Redirect(elsewhere.clone()),
    ));
    let report = run(
        &mut world,
        &alice,
        &p,
        retitle(&p, &id, &etag),
        None,
        deadline,
    );
    assert_eq!(code(&report), Some(FailureCode::ProviderRejected));
    world.fake.arm(Fault::on(
        "GET",
        "/calendar/v3/calendars/",
        FaultAction::Redirect(elsewhere),
    ));
    let read = world
        .as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline))
        .expect_err("refused");
    assert_eq!(read_code(&read), Some(FailureCode::ProviderRejected));
    assert!(witness.accept().is_err(), "nobody followed the redirect");
    assert_eq!(world.fake.log().len(), 2);

    let unusable = [
        FaultAction::TooLong(300 * 1024),
        FaultAction::Gzip,
        FaultAction::ContentType("text/html; charset=UTF-8".into()),
        FaultAction::NotJson,
    ];
    for fault in unusable {
        let (id, etag) = meeting(&world, &p, "meeting1");
        world
            .fake
            .arm(Fault::on("PATCH", "/calendar/v3/calendars/", fault.clone()));
        let report = run(
            &mut world,
            &alice,
            &p,
            retitle(&p, &id, &etag),
            None,
            deadline,
        );
        assert_eq!(report.outcome, ChangeOutcome::NoAnswer, "{fault:?}");
        world
            .fake
            .arm(Fault::on("GET", "/calendar/v3/calendars/", fault.clone()));
        let read = world.as_account(&alice, |g| g.observe(&alice.id, &p, &id, false, deadline));
        assert_eq!(read, Err(ReadFailure::transient()), "{fault:?}");
    }
    world.check_log();
}

/// Every fault the fake offers for a Calendar request (14.2).
fn every_fault(world: &World, target: &CalendarTarget) -> Vec<FaultAction> {
    let calendar = target.provider_id.as_str().to_owned();
    let _ = world;
    vec![
        FaultAction::error(400, "invalid"),
        FaultAction::error(403, "rateLimitExceeded"),
        FaultAction::error_retry_after(429, "userRateLimitExceeded", "7"),
        FaultAction::error(403, "quotaExceeded"),
        FaultAction::error(403, "forbiddenForNonOrganizer"),
        FaultAction::error(403, "insufficientPermissions"),
        FaultAction::error(403, "forbidden"),
        FaultAction::error(404, "notFound"),
        FaultAction::error(409, "conflict"),
        FaultAction::error(410, "deleted"),
        FaultAction::error(412, "conditionNotMet"),
        FaultAction::error(500, "backendError"),
        FaultAction::error(503, "backendError"),
        FaultAction::ApplyThenClose,
        FaultAction::apply_then_503(),
        FaultAction::CloseWithoutApplying,
        FaultAction::Delay(Duration::from_millis(20)),
        FaultAction::ApplyLate(LateTrigger::Manual),
        FaultAction::ApplyLate(LateTrigger::AfterReads(1)),
        FaultAction::NotJson,
        FaultAction::TooLong(300 * 1024),
        FaultAction::ContentType("text/html".into()),
        FaultAction::Gzip,
        FaultAction::Redirect("http://127.0.0.1:9/x".into()),
        FaultAction::ExpiredAccessToken,
        FaultAction::HideInsertForReads(1),
        FaultAction::CutAttendees,
        FaultAction::Json(200, json!({"id": "otherevent", "etag": "\"1\""})),
        FaultAction::Json(200, json!("not an event")),
        FaultAction::Raw(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
        FaultAction::change_event(
            &calendar,
            "meeting1",
            json!({"summary": "Changed by someone"}),
        ),
        FaultAction::before(|s| s.set_deleted_event_get(fabric_google_fake::DeletedEventGet::Gone)),
    ]
}

fn every_tool(world: &World, target: &CalendarTarget, n: u32) -> Vec<(Action, Option<String>)> {
    let (id, etag) = meeting(world, target, "meeting1");
    vec![
        (create(target), Some(reserved(n))),
        (retitle(target, &id, &etag), None),
        (
            invitation(
                target,
                event_ref(&id, &etag),
                &["dan@example.com"],
                SendUpdates::All,
            ),
            None,
        ),
        (
            response(
                target,
                event_ref(&id, &etag),
                ProposedResponse::Accepted,
                SendUpdates::ExternalOnly,
            ),
            None,
        ),
        (
            delete(target, event_ref(&id, &etag), SendUpdates::None),
            None,
        ),
    ]
}

/// g40 and g41: for every tool and every fault, at most one POST, PATCH or DELETE per order;
/// every PATCH and DELETE carries `If-Match` with one strong tag, and every changing request
/// an explicit `sendUpdates`. g16's Calendar half: no formatted value reached holds a token, a
/// calendar id, an event id or an event title.
#[test]
fn every_order_makes_at_most_one_changing_request_whatever_the_fault() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();
    let faults = every_fault(&world, &p);
    let mut n = 0;
    let mut tokens = vec![world.tokens.token(&alice.id)];
    for fault in &faults {
        for tool in 0..5 {
            n += 1;
            let (action, id) = every_tool(&world, &p, n).swap_remove(tool);
            world
                .fake
                .arm(Fault::matching(Match::any(), fault.clone()).always());
            let start = world.fake.log().len();
            let order = ChangeOrder::new(action.clone(), id.clone(), 1);
            world
                .formatted
                .push(format!("{order:?} {:?}", Effect::new(action, id)));
            let report = run(
                &mut world,
                &alice,
                &p,
                order.effect.action.clone(),
                order.effect.reserved_id.clone(),
                deadline,
            );
            world.formatted.push(format!("{report:?}"));
            world.fake.disarm_all();
            world.fake.apply_late_changes();
            let changing = world
                .log_since(start)
                .iter()
                .filter(|e| e.is_calendar_change())
                .count();
            assert!(
                changing <= 1,
                "{fault:?} tool {tool}: {changing} changing requests"
            );
            renew_token(&world, &alice);
            tokens.push(world.tokens.token(&alice.id));
        }
        // Reads under the same fault, for g16.
        world
            .fake
            .arm(Fault::matching(Match::any(), fault.clone()).always());
        let observed = world.as_account(&alice, |g| {
            g.observe(&alice.id, &p, "meeting1", true, deadline)
        });
        let listed = world.as_account(&alice, |g| g.list_calendars(&alice.id, deadline));
        let q = query(std::slice::from_ref(&p), &[EventField::Title]);
        if let Err(failure) =
            world.as_account(&alice, |g| g.read_events(&alice.id, &q, deadline, &|| true))
        {
            world.formatted.push(format!("{failure:?}"));
        }
        world.formatted.push(format!("{observed:?} {listed:?}"));
        world.fake.disarm_all();
        world.fake.apply_late_changes();
        renew_token(&world, &alice);
        tokens.push(world.tokens.token(&alice.id));
    }
    let log = world.fake.log();
    assert!(
        log.calendar_changes().len() > 100,
        "the orders reached the fake"
    );
    // g41.
    for entry in log.calendar_changes().entries() {
        if matches!(entry.method.as_str(), "PATCH" | "DELETE") {
            let tag = entry
                .if_match
                .as_deref()
                .expect("If-Match on every PATCH and DELETE");
            assert!(
                tag.len() >= 3 && tag.starts_with('"') && tag.ends_with('"'),
                "{tag}"
            );
            assert!(!tag[1..tag.len() - 1].contains('"'), "one tag: {tag}");
            assert!(!tag.starts_with("W/") && tag != "*");
        }
        let updates = entry.query_values("sendUpdates");
        assert_eq!(updates.len(), 1, "an explicit sendUpdates");
        assert!(matches!(updates[0], "all" | "externalOnly" | "none"));
    }
    // g16, the Calendar half.
    world.formatted.push(format!(
        "{:?} {:?} {:?}",
        world.google,
        p,
        query(std::slice::from_ref(&p), &[EventField::Title])
    ));
    let text = world.formatted.join("\n");
    let mut secrets: Vec<String> = tokens;
    secrets.extend([
        "alice@example.com".to_owned(),
        "meeting1".to_owned(),
        TITLE.to_owned(),
        "Renamed".to_owned(),
        reserved(1),
    ]);
    for secret in &secrets {
        assert!(
            !text.contains(secret.as_str()),
            "a formatted value holds a secret"
        );
    }
    assert!(text.contains("Rejected") && text.contains("NoAnswer") && text.contains("Applied"));
    world.check_log();
}

/// g42: an event id of `.`, `..`, `%2e%2e`, or with `/`, `?`, `#` or a space makes no request;
/// a revision of `*`, `W/"x"` or a list makes no request; a calendar id with `#` and `@`
/// reaches the fake as one path part.
#[test]
fn a_bad_event_id_or_revision_makes_no_request_and_a_calendar_id_is_one_path_part() {
    let (mut world, alice) = World::with_alice();
    let p = alice.primary.clone();
    let deadline = world.deadline();
    let good_rev = "\"1\"";
    for bad in [".", "..", "%2e%2e", "a/b", "a?b", "a#b", "..%2f.."] {
        for action in [
            retitle(&p, bad, good_rev),
            invitation(
                &p,
                event_ref(bad, good_rev),
                &["dan@example.com"],
                SendUpdates::All,
            ),
            response(
                &p,
                event_ref(bad, good_rev),
                ProposedResponse::Accepted,
                SendUpdates::All,
            ),
            delete(&p, event_ref(bad, good_rev), SendUpdates::All),
        ] {
            let report = run(&mut world, &alice, &p, action.clone(), None, deadline);
            assert_eq!(code(&report), Some(FailureCode::InvalidArguments), "{bad}");
            let back = world.as_account(&alice, |g| {
                g.reconcile_read(&alice.id, &p, &Effect::new(action, None), deadline)
            });
            assert_eq!(
                read_code(&back.expect_err("refused")),
                Some(FailureCode::InvalidArguments)
            );
        }
        let observed = world.as_account(&alice, |g| g.observe(&alice.id, &p, bad, false, deadline));
        assert_eq!(
            read_code(&observed.expect_err("refused")),
            Some(FailureCode::InvalidArguments)
        );
    }
    for bad in ["a b", "", "a\u{e9}"] {
        let observed = world.as_account(&alice, |g| g.observe(&alice.id, &p, bad, false, deadline));
        assert_eq!(
            read_code(&observed.expect_err("refused")),
            Some(FailureCode::InvalidArguments)
        );
    }
    for bad_rev in ["*", "W/\"x\"", "\"a\",\"b\"", "x"] {
        for action in [
            retitle(&p, "meeting1", bad_rev),
            invitation(
                &p,
                event_ref("meeting1", bad_rev),
                &["dan@example.com"],
                SendUpdates::All,
            ),
            response(
                &p,
                event_ref("meeting1", bad_rev),
                ProposedResponse::Accepted,
                SendUpdates::All,
            ),
            delete(&p, event_ref("meeting1", bad_rev), SendUpdates::All),
        ] {
            let report = run(&mut world, &alice, &p, action, None, deadline);
            assert_eq!(
                code(&report),
                Some(FailureCode::InvalidArguments),
                "{bad_rev}"
            );
        }
    }
    assert!(world.fake.log().is_empty(), "no request at all");

    let team = world.calendar(
        &alice,
        CalendarEntry::new("team#ops@group.calendar.google.com", "owner").time_zone(ZONE),
    );
    put(&world, &team, timed("teamevent", "Team", 9, 10));
    let observed = world.as_account(&alice, |g| {
        g.observe(&alice.id, &team, "teamevent", false, deadline)
    });
    assert!(matches!(observed, Ok(Observed::Found { .. })));
    let entry = world.fake.log().last().cloned().expect("a request");
    assert_eq!(
        entry.segments,
        [
            "calendar",
            "v3",
            "calendars",
            "team#ops@group.calendar.google.com",
            "events",
            "teamevent"
        ]
    );
    assert!(
        entry
            .raw_path
            .contains("team%23ops%40group.calendar.google.com")
    );
    world.check_log();
}
