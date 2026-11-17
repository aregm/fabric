//! Changing an event (6.6 to 6.8): create, update, invitation, response, delete.
//!
//! One order makes at most one request that can change Google (PD13); the reads before an
//! invitation and a response come first. Ids and revisions are checked before any request
//! (5.3, PD32); a failure there is Rejected `invalid_arguments` and nothing is sent. Every
//! PATCH and DELETE carries `If-Match` with the approved revision (PD14) and every changing
//! request an explicit `sendUpdates`. With fewer than 10 s of the deadline left when the
//! changing request is due, nothing is sent and the order is Transient (6.10).

use fabric_desktop_contracts::agent::{Action, ResponseTarget};
use fabric_desktop_contracts::common::{FailureCode, SendUpdates};
use fabric_desktop_contracts::scalar::{Id, ProviderRef};
use serde_json::Value;

use super::events::EventRead;
use super::{CalendarTarget, ChangeOrder, Google};
use crate::http::ids::{EventId, ReservedEventId, Revision};
use crate::http::{BodyBound, Exchange, Method, Request, Target};
use crate::outcome::{
    ChangeKind, ChangeOutcome, ChangeReport, Notification, ReadFailure, Refusal, Sentence,
    change_access_failure, change_refusal,
};
use crate::secret::Secret;
use crate::time::parse_rfc3339_ms;
use crate::translate::{
    GoogleEvent, create_body, invitation_body, matches, own_entry, response_body,
    response_confirmed, response_event_id, update_body,
};

/// The least time of the deadline that must be left when the changing request is due (6.10):
/// with less than the connect limit left, a connection cut off by the deadline could not be
/// told from a request that was sent.
pub const MIN_CHANGE_TIME_MS: u64 = crate::http::CONNECT_LIMIT_MS;

/// The `sendUpdates` value (6.9).
const fn send_updates(value: SendUpdates) -> &'static str {
    match value {
        SendUpdates::All => "all",
        SendUpdates::ExternalOnly => "externalOnly",
        SendUpdates::None => "none",
    }
}

const fn notification(value: SendUpdates) -> Notification {
    match value {
        SendUpdates::None => Notification::NotRequested,
        SendUpdates::All | SendUpdates::ExternalOnly => Notification::Sent,
    }
}

fn rejected(code: FailureCode, sentence: Sentence) -> ChangeReport {
    ChangeReport::plain(ChangeOutcome::rejected(code, sentence))
}

fn invalid(sentence: Sentence) -> ChangeReport {
    rejected(FailureCode::InvalidArguments, sentence)
}

/// A failed read before a change is the change's outcome: nothing changing was sent.
fn from_read(failure: ReadFailure) -> ChangeReport {
    match failure {
        ReadFailure::Transient { retry_after_ms } => {
            ChangeReport::plain(ChangeOutcome::Transient { retry_after_ms })
        }
        ReadFailure::Rejected {
            code,
            explanation,
            side,
        } => ChangeReport {
            outcome: ChangeOutcome::Rejected { code, explanation },
            side,
        },
        ReadFailure::Stopped => ChangeReport::plain(ChangeOutcome::transient()),
    }
}

/// The checked event id and revision of an approved reference (5.3).
fn checked(
    event_id: &ProviderRef,
    revision: &ProviderRef,
) -> Result<(EventId, Revision), ChangeReport> {
    let id = EventId::new(event_id.as_str()).map_err(|_| invalid(Sentence::EventIdForm))?;
    let revision = Revision::new(revision.as_str()).map_err(|_| invalid(Sentence::RevisionForm))?;
    Ok((id, revision))
}

/// The request an order sends, before the reads that complete its body.
enum Plan {
    Create(ReservedEventId),
    Update(EventId, Revision),
    Invitation(EventId, Revision),
    Response(EventId, Revision, Option<i64>),
    Delete(EventId, Revision),
}

impl Google {
    /// Carries out one changing order (6.6 to 6.8) and reports its outcome (6.10) with a result
    /// for the account when there is one (PO6). The order must name `account` and the target's
    /// calendar; otherwise, and for an action that changes no calendar, it is Rejected
    /// `invalid_arguments` with no request.
    pub fn change(
        &mut self,
        account: &Id,
        target: &CalendarTarget,
        order: &ChangeOrder,
        deadline_ms: u64,
    ) -> ChangeReport {
        match self.change_inner(account, target, order, deadline_ms) {
            Ok(report) | Err(report) => report,
        }
    }

    fn change_inner(
        &mut self,
        account: &Id,
        target: &CalendarTarget,
        order: &ChangeOrder,
        deadline_ms: u64,
    ) -> Result<ChangeReport, ChangeReport> {
        let action = &order.effect.action;
        let (action_account, action_calendar) = match action {
            Action::ProposeCreate(a) => (&a.account_id, &a.calendar_id),
            Action::ProposeUpdate(a) => (&a.account_id, &a.calendar_id),
            Action::ProposeInvitation(a) => (&a.account_id, &a.calendar_id),
            Action::ProposeResponse(a) => (&a.account_id, &a.calendar_id),
            Action::ProposeDelete(a) => (&a.account_id, &a.calendar_id),
            _ => return Err(invalid(Sentence::OrderMismatch)),
        };
        if action_account != account
            || &target.calendar.account_id != account
            || action_calendar != &target.calendar.calendar_id
        {
            return Err(invalid(Sentence::OrderMismatch));
        }

        // The checks of 5.3, before any request.
        let plan = match action {
            Action::ProposeCreate(_) => Plan::Create(
                ReservedEventId::new(order.effect.reserved_id.as_deref().unwrap_or(""))
                    .map_err(|_| invalid(Sentence::ReservedIdForm))?,
            ),
            Action::ProposeUpdate(a) => {
                let (id, rev) = checked(&a.event.event_id, &a.event.revision)?;
                Plan::Update(id, rev)
            }
            Action::ProposeInvitation(a) => {
                let (id, rev) = checked(&a.event.event_id, &a.event.revision)?;
                Plan::Invitation(id, rev)
            }
            Action::ProposeResponse(a) => {
                let (event_id, revision) = response_event_id(&a.target);
                let (id, rev) = checked(event_id, revision)?;
                let original = match &a.target {
                    ResponseTarget::Event(_) => None,
                    ResponseTarget::Occurrence(o) => Some(
                        parse_rfc3339_ms(o.original_start_time.date_time.as_str())
                            .map_err(|_| invalid(Sentence::InvalidRequest))?,
                    ),
                };
                Plan::Response(id, rev, original)
            }
            Action::ProposeDelete(a) => {
                let (id, rev) = checked(&a.event.event_id, &a.event.revision)?;
                Plan::Delete(id, rev)
            }
            _ => return Err(invalid(Sentence::OrderMismatch)),
        };

        let token = self
            .tokens
            .access_token(account, deadline_ms)
            .map_err(change_access_failure)?;

        // The request, and for an invitation and a response the event as read before it.
        let calendar = target.provider_id.clone();
        let (request, kind, expected_id, read, sent_updates) = match (&plan, action) {
            (Plan::Create(id), Action::ProposeCreate(a)) => {
                let request =
                    Request::new(Method::Post, Target::Events(calendar), BodyBound::Medium)
                        .query("sendUpdates", "none")
                        .json(&create_body(&a.event, id));
                (
                    request,
                    ChangeKind::Create,
                    id.as_str().to_owned(),
                    None,
                    SendUpdates::None,
                )
            }
            (Plan::Update(id, rev), Action::ProposeUpdate(a)) => {
                let request = patch(&calendar, id, rev, a.send_updates)
                    .and_then(|r| r.json(&update_body(&a.changes)));
                (
                    request,
                    ChangeKind::Patch,
                    id.as_str().to_owned(),
                    None,
                    a.send_updates,
                )
            }
            (Plan::Invitation(id, rev), Action::ProposeInvitation(a)) => {
                let read = self.read_before(account, &token, target, id, rev, None, deadline_ms)?;
                let own = own_entry(&read, target);
                let request = patch(&calendar, id, rev, a.send_updates)
                    .and_then(|r| r.json(&invitation_body(&read, &a.attendees, own)));
                (
                    request,
                    ChangeKind::Patch,
                    id.as_str().to_owned(),
                    Some(read),
                    a.send_updates,
                )
            }
            (Plan::Response(id, rev, original), Action::ProposeResponse(a)) => {
                let read =
                    self.read_before(account, &token, target, id, rev, *original, deadline_ms)?;
                // An answer needs an own entry (6.1, 6.7): another person's invitation on a
                // calendar that person shared is never answered.
                let Some(own) = own_entry(&read, target) else {
                    return Err(rejected(
                        FailureCode::ProviderRejected,
                        Sentence::NotInvited,
                    ));
                };
                let request = patch(&calendar, id, rev, a.send_updates)
                    .and_then(|r| r.json(&response_body(&read, own, a.response)));
                (
                    request,
                    ChangeKind::Patch,
                    id.as_str().to_owned(),
                    Some(read),
                    a.send_updates,
                )
            }
            (Plan::Delete(id, rev), Action::ProposeDelete(a)) => {
                let request = Request::new(
                    Method::Delete,
                    Target::Event(calendar, id.clone()),
                    BodyBound::Small,
                )
                .query("sendUpdates", send_updates(a.send_updates))
                .if_match(rev);
                (
                    request,
                    ChangeKind::Delete,
                    id.as_str().to_owned(),
                    None,
                    a.send_updates,
                )
            }
            _ => return Err(invalid(Sentence::OrderMismatch)),
        };
        // The request is formed from checked values; a failure here means nothing exists to
        // send.
        let request = request
            .and_then(|r| r.bearer(&token))
            .map_err(|_| ChangeReport::plain(ChangeOutcome::transient()))?;

        // 6.10: fewer than 10 s left when the changing request is due: Transient, nothing sent.
        if deadline_ms.saturating_sub(self.now_ms()) < MIN_CHANGE_TIME_MS {
            return Err(ChangeReport::plain(ChangeOutcome::transient()));
        }

        // The one changing request of this order (PD13).
        let answer = match self.transport.send(request, deadline_ms) {
            Exchange::NothingSent(_) => return Ok(ChangeOutcome::transient().into()),
            Exchange::SentUnanswered(_) => return Ok(ChangeOutcome::NoAnswer.into()),
            Exchange::Answer(answer) => answer,
        };
        if !answer.is_success() {
            if answer.status() == 401 {
                self.tokens.drop_cached(account);
            }
            return Ok(change_refusal(kind, Refusal::of(&answer)));
        }
        let notification = match kind {
            ChangeKind::Create => Notification::NotRequested,
            ChangeKind::Patch | ChangeKind::Delete => notification(sent_updates),
        };
        if kind == ChangeKind::Delete {
            // 6.8: 204 or 200; an answer with no body, so not `confirmed` (section 7).
            return Ok(if matches!(answer.status(), 200 | 204) {
                ChangeOutcome::Applied {
                    event: None,
                    confirmed: false,
                    notification,
                }
            } else {
                ChangeOutcome::NoAnswer
            }
            .into());
        }
        // 6.6, 6.7: 2xx with the event; another id, or a body that is not an event, is
        // NoAnswer.
        let Some(after) = answer
            .json()
            .ok()
            .and_then(|body: Value| GoogleEvent::read(&body).ok())
            .filter(|event| event.id == expected_id && !event.cancelled)
        else {
            return Ok(ChangeOutcome::NoAnswer.into());
        };
        let (Ok(event_id), Ok(revision)) = (
            ProviderRef::new(after.id.as_str()),
            ProviderRef::new(after.etag.as_str()),
        ) else {
            return Ok(ChangeOutcome::NoAnswer.into());
        };
        let confirmed = match (action, &read) {
            (Action::ProposeResponse(_), Some(read)) => {
                response_confirmed(action, read, &after, target)
            }
            _ => matches(action, &after, target),
        };
        Ok(ChangeOutcome::Applied {
            event: Some(fabric_desktop_contracts::common::EventRef { event_id, revision }),
            confirmed,
            notification,
        }
        .into())
    }

    /// The read before an invitation or a response (6.7): it must return the approved
    /// revision and must not have `attendeesOmitted: true`; for one occurrence its original
    /// start must be the approved instant (CD-Q05b). Otherwise Rejected without a PATCH.
    #[allow(clippy::too_many_arguments)]
    fn read_before(
        &mut self,
        account: &Id,
        token: &Secret,
        target: &CalendarTarget,
        id: &EventId,
        revision: &Revision,
        original_start_ms: Option<i64>,
        deadline_ms: u64,
    ) -> Result<GoogleEvent, ChangeReport> {
        let event = match self
            .get_event(account, token, target, id, deadline_ms)
            .map_err(from_read)?
        {
            EventRead::Absent => {
                return Err(rejected(FailureCode::RevisionConflict, Sentence::EventGone));
            }
            EventRead::Event(event) if event.cancelled => {
                return Err(rejected(FailureCode::RevisionConflict, Sentence::EventGone));
            }
            EventRead::Event(event) => *event,
        };
        if event.etag != revision.as_str() {
            return Err(rejected(
                FailureCode::RevisionConflict,
                Sentence::EventChanged,
            ));
        }
        if event.attendees_omitted {
            return Err(invalid(Sentence::AttendeesCut));
        }
        if let Some(original) = original_start_ms
            && event.original_start_ms() != Some(original)
        {
            return Err(rejected(
                FailureCode::RevisionConflict,
                Sentence::EventChanged,
            ));
        }
        Ok(event)
    }
}

/// `PATCH /calendars/<id>/events/<event id>?sendUpdates=<approved>` with `If-Match` (6.7).
fn patch(
    calendar: &crate::http::ids::CalendarId,
    id: &EventId,
    revision: &Revision,
    updates: SendUpdates,
) -> Result<Request, crate::http::RequestError> {
    Request::new(
        Method::Patch,
        Target::Event(calendar.clone(), id.clone()),
        BodyBound::Medium,
    )
    .query("sendUpdates", send_updates(updates))
    .if_match(revision)
}
