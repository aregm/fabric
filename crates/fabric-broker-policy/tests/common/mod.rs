//! The context and action of the golden check theory `fixtures/Check_000001.thy`.

#![allow(dead_code, reason = "each test file uses a different part")]

use fabric_broker_policy::{
    Action, ActiveSession, Actor, Calendar, CalendarKey, Context, Duration, Event, EventField,
    Grant, PendingEffect, Profile, Recipient, RecipientRule, RecipientSet, SendUpdates, Tool,
    Window,
};

pub const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub fn recipient(address: &str, domain: &str) -> Recipient {
    Recipient {
        address: address.to_owned(),
        domain: domain.to_owned(),
    }
}

pub fn context() -> Context {
    Context {
        actor: Actor::Agent("agent-1".to_owned()),
        paused: false,
        now_ms: 1_700_000_000_000,
        session: Some(ActiveSession {
            id: "session-1".to_owned(),
            last_active_ms: 1_699_999_990_000,
        }),
        profile: Profile::FLEXIBLE,
        connected: vec!["acct-1".to_owned()],
        calendars: vec![
            Calendar {
                account: "acct-1".to_owned(),
                id: "drafts".to_owned(),
                writable: true,
                drafts: true,
            },
            Calendar {
                account: "acct-1".to_owned(),
                id: "team".to_owned(),
                writable: true,
                drafts: false,
            },
        ],
        warning_calendars: vec![CalendarKey::new("acct-1", "private")],
        grants: vec![
            Grant {
                id: "g-invite".to_owned(),
                tools: vec![Tool::Invite, Tool::Update],
                duration: Duration::Session {
                    session_id: "session-1".to_owned(),
                },
                ended: false,
                calendars: vec![CalendarKey::new("acct-1", "team")],
                window: Some(Window {
                    start_ms: -5,
                    end_ms: 1000,
                }),
                fields: vec![EventField::Time, EventField::Attendees],
                recipients: Some(RecipientRule::Sets(vec!["set-team".to_owned()])),
                send_updates: Some(SendUpdates::NotifyAll),
            },
            Grant {
                id: "g-once".to_owned(),
                tools: Vec::new(),
                duration: Duration::OneRequest {
                    digest: DIGEST.to_owned(),
                    used: false,
                },
                ended: false,
                calendars: Vec::new(),
                window: None,
                fields: Vec::new(),
                recipients: Some(RecipientRule::Exact(vec!["bob@example.com".to_owned()])),
                send_updates: None,
            },
            Grant {
                id: "g-standing".to_owned(),
                tools: vec![Tool::List],
                duration: Duration::UntilRevoked,
                ended: true,
                calendars: Vec::new(),
                window: None,
                fields: Vec::new(),
                recipients: None,
                send_updates: None,
            },
        ],
        recipient_sets: vec![RecipientSet {
            id: "set-team".to_owned(),
            members: vec!["bob@example.com".to_owned()],
            domains: vec!["example.org".to_owned()],
        }],
        event: Some(Event {
            id: "evt1".to_owned(),
            revision: "7".to_owned(),
            fabric_created: true,
            recurring: false,
            default_type: true,
            timed: true,
            invitation: false,
            attendees: vec![recipient("bob@example.com", "example.com")],
            occurrence: None,
        }),
        digest: DIGEST.to_owned(),
        relied: vec!["g-invite".to_owned()],
        result_held: false,
        pending_effects: vec![PendingEffect {
            account: "acct-1".to_owned(),
            calendar: "team".to_owned(),
            event_id: "evt0".to_owned(),
        }],
        pause_epoch: 3,
        revocation_epoch: 5,
    }
}

pub fn action() -> Action {
    Action::ChangeInvitation {
        account: "acct-1".to_owned(),
        calendar: "team".to_owned(),
        event_id: "evt1".to_owned(),
        revision: "7".to_owned(),
        attendees: vec![
            recipient("bob@example.com", "example.com"),
            recipient("carol@example.org", "example.org"),
        ],
        send_updates: SendUpdates::NotifyAll,
    }
}
