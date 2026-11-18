//! Changing an event (6.6 to 6.8): create, update, invitation, response, delete.
//!
//! One order makes at most one request that can change Google (PD13); the reads before an
//! invitation and a response come first. Ids and revisions are checked before any request
//! (5.3, PD32), as are an update that sets nothing and an attendee list that names one
//! address twice; a failure there is Rejected `invalid_arguments` and nothing is sent. The
//! answer to a POST or a PATCH is read with the large bound: one event may carry many
//! attendees. Every
//! PATCH and DELETE carries `If-Match` with the approved revision (PD14) and every changing
//! request an explicit `sendUpdates`. With fewer than 10 s of the deadline left when the
//! changing request is due, nothing is sent and the order is Transient (6.10).
//!
//! A delete that Google answered 200 or 204 is followed by one read of the same event, which
//! sets only `confirmed` (18.7 point 1). A create, an update, an invitation and a response
//! answered 200 with the right id are `Applied` whatever the event holds; when it is not what
//! was approved, `differs` names the first member that is not (18.7 point 2; 6.12).

use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::common::{FailureCode, SendUpdates};
use fabric_desktop_contracts::scalar::{Id, ProviderRef};
use serde_json::Value;

use super::events::EventRead;
use super::{CalendarTarget, ChangeOrder, Google, check_effect};
use crate::http::ids::{EventId, Revision};
use crate::http::{Answer, BodyBound, Exchange, Method, Request, Target};
use crate::outcome::{
    ChangeKind, ChangeOutcome, ChangeReport, Notification, ReadFailure, Refusal, Sentence,
    change_access_failure, change_refusal,
};
use crate::secret::Secret;
use crate::translate::{
    GoogleEvent, changes_nothing, create_body, differs, invitation_body, names_an_address_twice,
    occurrence_differs, own_entry, response_body, response_differs, update_body,
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
        // The checks of 5.3 and of the order's fit to its target, before any request (the same
        // check `reconcile_read` makes).
        let ids = check_effect(account, target, &order.effect).map_err(invalid)?;
        match action {
            Action::ProposeUpdate(a) if changes_nothing(&a.changes) => {
                return Err(invalid(Sentence::NothingToChange));
            }
            Action::ProposeInvitation(a)
                if names_an_address_twice(a.attendees.iter().map(|e| e.as_str())) =>
            {
                return Err(invalid(Sentence::AddressTwice));
            }
            _ => {}
        }

        let token = self
            .tokens
            .access_token(account, deadline_ms)
            .map_err(change_access_failure)?;

        // The request, and for an invitation and a response the event as read before it.
        let calendar = target.provider_id.clone();
        let id = &ids.event_id;
        let (request, kind, read, sent_updates) = match (action, ids.revision.as_ref()) {
            (Action::ProposeCreate(a), None) => {
                let request =
                    Request::new(Method::Post, Target::Events(calendar), BodyBound::Large)
                        .query("sendUpdates", "none")
                        .json(&create_body(&a.event, id));
                (request, ChangeKind::Create, None, SendUpdates::None)
            }
            (Action::ProposeUpdate(a), Some(rev)) => {
                let request = patch(&calendar, id, rev, a.send_updates)
                    .and_then(|r| r.json(&update_body(&a.changes)));
                (request, ChangeKind::Patch, None, a.send_updates)
            }
            (Action::ProposeInvitation(a), Some(rev)) => {
                let read = self.read_before(account, &token, target, id, rev, None, deadline_ms)?;
                let own = own_entry(&read, target);
                let request = patch(&calendar, id, rev, a.send_updates)
                    .and_then(|r| r.json(&invitation_body(&read, &a.attendees, own)));
                (request, ChangeKind::Patch, Some(read), a.send_updates)
            }
            (Action::ProposeResponse(a), Some(rev)) => {
                let read = self.read_before(
                    account,
                    &token,
                    target,
                    id,
                    rev,
                    ids.original_start_ms,
                    deadline_ms,
                )?;
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
                (request, ChangeKind::Patch, Some(read), a.send_updates)
            }
            (Action::ProposeDelete(a), Some(rev)) => {
                let request = Request::new(
                    Method::Delete,
                    Target::Event(calendar, id.clone()),
                    BodyBound::Small,
                )
                .query("sendUpdates", send_updates(a.send_updates))
                .if_match(rev);
                (request, ChangeKind::Delete, None, a.send_updates)
            }
            _ => return Err(invalid(Sentence::OrderMismatch)),
        };
        let expected_id = id.as_str();
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
            // 6.8: 204 or 200 is Applied. `confirmed` comes from one read of the event in the
            // same order (18.7 point 1), which never changes the class.
            if !matches!(answer.status(), 200 | 204) {
                return Ok(ChangeOutcome::NoAnswer.into());
            }
            let confirmed = self.delete_read_back(account, &token, target, id, deadline_ms);
            return Ok(ChangeOutcome::Applied {
                event: None,
                confirmed,
                differs: None,
                notification,
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
        // 6.12; 18.7 point 2: a yes with the right id is Applied whatever the event holds;
        // the comparison says whether it is what was approved and, if not, names the first
        // member that is not.
        let differs = match (action, &read) {
            (Action::ProposeResponse(_), Some(read)) => {
                response_differs(action, read, &after, target)
            }
            _ => differs(action, &after, target),
        }
        .or_else(|| occurrence_differs(&after, ids.original_start_ms));
        Ok(ChangeOutcome::Applied {
            event: Some(fabric_desktop_contracts::common::EventRef { event_id, revision }),
            confirmed: differs.is_none(),
            differs,
            notification,
        }
        .into())
    }

    /// The read after a delete that Google answered 200 or 204 (6.8; 18.7 point 1): one `GET` of
    /// the same event, with the access token the DELETE used, inside what is left of the
    /// order's deadline, with no refresh and no retry. It is a read, so the order still makes
    /// one changing request (PD13). Nothing is sent when no time is left. It gives only whether
    /// the deletion is in effect ([`delete_confirmed`]); nothing of the event is kept.
    fn delete_read_back(
        &mut self,
        account: &Id,
        token: &Secret,
        target: &CalendarTarget,
        id: &EventId,
        deadline_ms: u64,
    ) -> bool {
        if self.now_ms() >= deadline_ms {
            return false;
        }
        let request = Request::new(
            Method::Get,
            Target::Event(target.provider_id.clone(), id.clone()),
            BodyBound::Large,
        )
        .query("timeZone", target.time_zone.as_str())
        .bearer(token);
        let Ok(request) = request else {
            return false;
        };
        let Exchange::Answer(answer) = self.transport.send(request, deadline_ms) else {
            return false;
        };
        if answer.status() == 401 {
            self.tokens.drop_cached(account);
        }
        delete_confirmed(&answer, id)
    }

    /// The read before an invitation or a response (6.7): it must return the approved
    /// revision and must not have `attendeesOmitted: true`; for one occurrence its original
    /// start must be the approved instant (CD-Q05b). Otherwise Rejected without a PATCH. An
    /// event that cannot be read is Rejected too, and no PATCH is sent.
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

/// Whether the answer to the read after a delete shows the deletion in effect (6.8; 18.7
/// point 1): 404, 410, or 200 with a readable event whose `id` is the one asked for and whose
/// `status` is `cancelled`, at any revision. Everything else is `false`: a live event, an
/// event with another id, a body that cannot be read, and every other status. Its own
/// mapping, apart from the read before a change and from `observe`: it gives a bool and
/// nothing of the event.
fn delete_confirmed(answer: &Answer, id: &EventId) -> bool {
    match answer.status() {
        404 | 410 => true,
        200 => answer
            .json()
            .ok()
            .and_then(|body: Value| GoogleEvent::read(&body).ok())
            .is_some_and(|event| event.id == id.as_str() && event.cancelled),
        _ => false,
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
        BodyBound::Large,
    )
    .query("sendUpdates", send_updates(updates))
    .if_match(revision)
}
