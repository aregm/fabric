//! The context and the action as HOL terms of `Fabric_Broker.thy`.
//!
//! Every value that reaches a check theory goes through this module. Broker identifiers go
//! through [`fabric_policy::hol::identifier`], which admits only characters that cannot end a
//! HOL string literal. Text from outside becomes the list of its Unicode code points
//! (`chars = nat list`), never a HOL string, so no input can change the theory's syntax.

use fabric_policy::PolicyError;
use fabric_policy::hol::{identifier, int, note_codes};

use crate::model::{
    Action, ActiveSession, Calendar, CalendarKey, Context, Duration, Event, Grant, PendingEffect,
    Profile, Recipient, RecipientRule, RecipientSet, Row, Target, Window,
};

/// An identifier that is an action digest: exactly 64 lowercase hex characters.
pub fn digest(value: &str) -> Result<String, PolicyError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        identifier(value)
    } else {
        Err(PolicyError::Render(format!(
            "digest {value:?} must be 64 lowercase hex characters"
        )))
    }
}

/// Text from outside as `chars`: the list of its code points.
#[must_use]
pub fn chars(text: &str) -> String {
    note_codes(text)
}

const fn boolean(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

fn try_list<T>(
    items: &[T],
    render: impl Fn(&T) -> Result<String, PolicyError>,
) -> Result<String, PolicyError> {
    Ok(list(
        items.iter().map(render).collect::<Result<Vec<_>, _>>()?,
    ))
}

fn identifiers(values: &[String]) -> Result<String, PolicyError> {
    try_list(values, |value| identifier(value))
}

/// `None`, or `Some` applied to a term that is already atomic.
fn option(value: Option<String>) -> String {
    value.map_or_else(|| "None".to_owned(), |term| format!("Some {term}"))
}

fn record(fields: &[(&str, String)]) -> String {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("\\<lparr>{body}\\<rparr>")
}

/// A profile as a `profile` record, every field in the order of the `row` datatype.
#[must_use]
pub fn profile(value: &Profile) -> String {
    let fields: Vec<(&str, String)> = Row::ALL
        .iter()
        .map(|&row| {
            (
                row.profile_field(),
                value.field(row).hol_constructor().to_owned(),
            )
        })
        .collect();
    record(&fields)
}

fn calendar_key(value: &CalendarKey) -> Result<String, PolicyError> {
    Ok(format!(
        "({}, {})",
        identifier(&value.account)?,
        identifier(&value.calendar)?
    ))
}

fn recipient(value: &Recipient) -> String {
    record(&[
        ("rc_address", chars(&value.address)),
        ("rc_domain", chars(&value.domain)),
    ])
}

fn recipients(values: &[Recipient]) -> String {
    list(values.iter().map(recipient))
}

fn recipient_set(value: &RecipientSet) -> Result<String, PolicyError> {
    Ok(record(&[
        ("rs_id", identifier(&value.id)?),
        ("rs_members", list(value.members.iter().map(|m| chars(m)))),
        ("rs_domains", list(value.domains.iter().map(|d| chars(d)))),
    ]))
}

fn recipient_rule(value: &RecipientRule) -> Result<String, PolicyError> {
    Ok(match value {
        RecipientRule::Exact(addresses) => {
            format!("Exact {}", list(addresses.iter().map(|a| chars(a))))
        }
        RecipientRule::Sets(ids) => format!("Sets {}", identifiers(ids)?),
    })
}

fn duration(value: &Duration) -> Result<String, PolicyError> {
    Ok(match value {
        Duration::OneRequest { digest: d, used } => {
            format!("One_request {} {}", digest(d)?, boolean(*used))
        }
        Duration::Session { session_id } => format!("Session {}", identifier(session_id)?),
        Duration::UntilRevoked => "Until_revoked".to_owned(),
    })
}

fn window(value: &Window) -> String {
    format!("({}, {})", int(value.start_ms), int(value.end_ms))
}

fn grant(value: &Grant) -> Result<String, PolicyError> {
    Ok(record(&[
        ("g_id", identifier(&value.id)?),
        (
            "g_tools",
            list(value.tools.iter().map(|t| t.hol_constructor().to_owned())),
        ),
        ("g_duration", duration(&value.duration)?),
        ("g_ended", boolean(value.ended).to_owned()),
        ("g_calendars", try_list(&value.calendars, calendar_key)?),
        ("g_window", option(value.window.as_ref().map(window))),
        (
            "g_fields",
            list(value.fields.iter().map(|f| f.hol_constructor().to_owned())),
        ),
        (
            "g_recipients",
            option(
                value
                    .recipients
                    .as_ref()
                    .map(recipient_rule)
                    .transpose()?
                    .map(|rule| format!("({rule})")),
            ),
        ),
        (
            "g_send_updates",
            option(value.send_updates.map(|s| s.hol_constructor().to_owned())),
        ),
    ]))
}

fn calendar(value: &Calendar) -> Result<String, PolicyError> {
    Ok(record(&[
        ("cal_account", identifier(&value.account)?),
        ("cal_id", identifier(&value.id)?),
        ("cal_writable", boolean(value.writable).to_owned()),
        ("cal_drafts", boolean(value.drafts).to_owned()),
    ]))
}

fn event(value: &Event) -> String {
    record(&[
        ("ev_id", chars(&value.id)),
        ("ev_revision", chars(&value.revision)),
        (
            "ev_fabric_created",
            boolean(value.fabric_created).to_owned(),
        ),
        ("ev_recurring", boolean(value.recurring).to_owned()),
        ("ev_default_type", boolean(value.default_type).to_owned()),
        ("ev_timed", boolean(value.timed).to_owned()),
        ("ev_invitation", boolean(value.invitation).to_owned()),
        ("ev_attendees", recipients(&value.attendees)),
        ("ev_occurrence", option(value.occurrence.map(int))),
    ])
}

fn session(value: &ActiveSession) -> Result<String, PolicyError> {
    Ok(format!(
        "({}, {})",
        identifier(&value.id)?,
        int(value.last_active_ms)
    ))
}

fn pending_effect(value: &PendingEffect) -> Result<String, PolicyError> {
    Ok(format!(
        "({}, {}, {})",
        identifier(&value.account)?,
        identifier(&value.calendar)?,
        chars(&value.event_id)
    ))
}

/// The context as a `broker_context` record, its fields in the theory's order.
pub fn context(value: &Context) -> Result<String, PolicyError> {
    let actor = match &value.actor {
        crate::model::Actor::Agent(agent) => format!("Agent {}", identifier(agent)?),
        crate::model::Actor::System => "System".to_owned(),
    };
    Ok(record(&[
        ("ctx_actor", actor),
        ("ctx_paused", boolean(value.paused).to_owned()),
        ("ctx_now", int(value.now_ms)),
        (
            "ctx_session",
            option(value.session.as_ref().map(session).transpose()?),
        ),
        ("ctx_profile", profile(&value.profile)),
        ("ctx_connected", identifiers(&value.connected)?),
        ("ctx_calendars", try_list(&value.calendars, calendar)?),
        (
            "ctx_warning_calendars",
            try_list(&value.warning_calendars, calendar_key)?,
        ),
        ("ctx_grants", try_list(&value.grants, grant)?),
        (
            "ctx_recipient_sets",
            try_list(&value.recipient_sets, recipient_set)?,
        ),
        (
            "ctx_event",
            option(value.event.as_ref().map(|e| format!("({})", event(e)))),
        ),
        ("ctx_digest", digest(&value.digest)?),
        ("ctx_relied", identifiers(&value.relied)?),
        ("ctx_result_held", boolean(value.result_held).to_owned()),
        (
            "ctx_pending_effects",
            try_list(&value.pending_effects, pending_effect)?,
        ),
        ("ctx_pause_epoch", value.pause_epoch.to_string()),
        ("ctx_revocation_epoch", value.revocation_epoch.to_string()),
    ]))
}

fn target(value: &Target) -> String {
    match value {
        Target::WholeEvent { event_id, revision } => {
            format!("Whole_event {} {}", chars(event_id), chars(revision))
        }
        Target::Occurrence {
            instance_id,
            original_start_ms,
            revision,
        } => format!(
            "Occurrence {} {} {}",
            chars(instance_id),
            int(*original_start_ms),
            chars(revision)
        ),
    }
}

/// The action as a term of the `action` datatype, its arguments in the theory's order.
pub fn action(value: &Action) -> Result<String, PolicyError> {
    Ok(match value {
        Action::ListCalendars => "List_calendars".to_owned(),
        Action::ReadAvailability {
            account,
            calendars,
            start_ms,
            end_ms,
        } => format!(
            "Read_availability {} {} {} {}",
            identifier(account)?,
            identifiers(calendars)?,
            int(*start_ms),
            int(*end_ms)
        ),
        Action::ReadEvents {
            account,
            calendars,
            start_ms,
            end_ms,
            fields,
        } => format!(
            "Read_events {} {} {} {} {}",
            identifier(account)?,
            identifiers(calendars)?,
            int(*start_ms),
            int(*end_ms),
            list(fields.iter().map(|f| f.hol_constructor().to_owned()))
        ),
        Action::CreateEvent { account, calendar } => format!(
            "Create_event {} {}",
            identifier(account)?,
            identifier(calendar)?
        ),
        Action::UpdateEvent {
            account,
            calendar,
            event_id,
            revision,
            send_updates,
        } => format!(
            "Update_event {} {} {} {} {}",
            identifier(account)?,
            identifier(calendar)?,
            chars(event_id),
            chars(revision),
            send_updates.hol_constructor()
        ),
        Action::ChangeInvitation {
            account,
            calendar,
            event_id,
            revision,
            attendees,
            send_updates,
        } => format!(
            "Change_invitation {} {} {} {} {} {}",
            identifier(account)?,
            identifier(calendar)?,
            chars(event_id),
            chars(revision),
            recipients(attendees),
            send_updates.hol_constructor()
        ),
        Action::Respond {
            account,
            calendar,
            target: answered,
            response,
            send_updates,
        } => format!(
            "Respond {} {} ({}) {} {}",
            identifier(account)?,
            identifier(calendar)?,
            target(answered),
            response.hol_constructor(),
            send_updates.hol_constructor()
        ),
        Action::DeleteEvent {
            account,
            calendar,
            event_id,
            revision,
            send_updates,
        } => format!(
            "Delete_event {} {} {} {} {}",
            identifier(account)?,
            identifier(calendar)?,
            chars(event_id),
            chars(revision),
            send_updates.hol_constructor()
        ),
        Action::Disclose(inner) => {
            let inner = action(inner)?;
            if inner.contains(' ') {
                format!("Disclose ({inner})")
            } else {
                format!("Disclose {inner}")
            }
        }
        Action::DiscloseWarning => "Disclose_warning".to_owned(),
        Action::DeleteCalendar { account, calendar } => format!(
            "Delete_calendar {} {}",
            identifier(account)?,
            identifier(calendar)?
        ),
        Action::ChangeSharing { account, calendar } => format!(
            "Change_sharing {} {}",
            identifier(account)?,
            identifier(calendar)?
        ),
        Action::SystemRefresh { account, calendar } => format!(
            "System_refresh {} {}",
            identifier(account)?,
            identifier(calendar)?
        ),
        Action::SystemReconcile {
            account,
            calendar,
            event_id,
        } => format!(
            "System_reconcile {} {} {}",
            identifier(account)?,
            identifier(calendar)?,
            chars(event_id)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{action, chars, digest};
    use crate::model::Action;

    #[test]
    fn a_digest_is_64_lowercase_hex_characters() {
        assert!(digest(&"a".repeat(64)).is_ok());
        assert!(digest(&"A".repeat(64)).is_err());
        assert!(digest(&"a".repeat(63)).is_err());
        assert!(digest("d-this").is_err());
    }

    #[test]
    fn text_becomes_code_points() {
        assert_eq!(chars("a\u{1F600}"), "[97, 128512]");
        assert_eq!(chars(""), "[]");
    }

    #[test]
    fn a_disclosure_parenthesizes_an_applied_constructor_only() {
        assert_eq!(
            action(&Action::ListCalendars.disclosed()).unwrap(),
            "Disclose List_calendars"
        );
        let create = Action::CreateEvent {
            account: "a".to_owned(),
            calendar: "c".to_owned(),
        };
        assert_eq!(
            action(&create.disclosed().disclosed()).unwrap(),
            "Disclose (Disclose (Create_event ''a'' ''c''))"
        );
    }
}
