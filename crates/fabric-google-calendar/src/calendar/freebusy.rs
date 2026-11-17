//! Free/busy (6.3).

use fabric_desktop_contracts::agent::{BusyInterval, ToolResult};
use fabric_desktop_contracts::common::{FailureCode, Window};
use fabric_desktop_contracts::scalar::{AnyList, Id};
use serde_json::{Value, json};

use super::{CalendarTarget, Google, ReadAnswer, check_targets};
use crate::http::{BodyBound, Method, Request, Target};
use crate::outcome::{ReadFailure, ReadKind, Sentence};
use crate::time::{format_timestamp, parse_rfc3339_ms};

/// The most calendars one query names (6.3; the contract's bound and Google's).
pub const MAX_FREE_BUSY_CALENDARS: usize = 50;

impl Google {
    /// `POST /freeBusy` for the targets over the window, in UTC (6.3). An `errors` entry for
    /// any asked calendar, or a missing calendar, fails the whole read (PD23). Each busy period
    /// becomes a [`BusyInterval`] under the broker's calendar id, in UTC with milliseconds, cut
    /// to the window; ordered by calendar (the order asked) and then by start. `observed_at` is
    /// the clock when the answer arrived.
    pub fn free_busy(
        &mut self,
        account: &Id,
        targets: &[CalendarTarget],
        window: &Window,
        deadline_ms: u64,
    ) -> Result<ToolResult, ReadFailure> {
        if targets.is_empty() || targets.len() > MAX_FREE_BUSY_CALENDARS {
            return Err(ReadFailure::rejected(
                FailureCode::InvalidArguments,
                Sentence::CalendarCount,
            ));
        }
        check_targets(account, &targets.iter().collect::<Vec<_>>())?;
        // The adapter finds the window unreadable itself; nothing was sent (PD27).
        let (Some(window_start), Some(window_end)) =
            (window.start.unix_millis(), window.end.unix_millis())
        else {
            return Err(ReadFailure::rejected(
                FailureCode::InvalidArguments,
                Sentence::TimeUnreadable,
            ));
        };
        let items: Vec<Value> = targets
            .iter()
            .map(|t| json!({"id": t.provider_id.as_str()}))
            .collect();
        let body = json!({
            "timeMin": window.start.as_str(),
            "timeMax": window.end.as_str(),
            "timeZone": "UTC",
            "items": items,
        });
        let Ok(request) =
            Request::new(Method::Post, Target::FreeBusy, BodyBound::Large).json(&body)
        else {
            return Err(ReadFailure::transient());
        };
        let token = self.read_token(account, deadline_ms)?;
        let ReadAnswer::Body(answer) =
            self.send_read(account, &token, request, deadline_ms, ReadKind::Calendar)?
        else {
            return Err(ReadFailure::transient());
        };
        let observed_at = format_timestamp(i64::try_from(self.now_ms()).unwrap_or(i64::MAX))
            .map_err(|_| ReadFailure::transient())?;
        let incomplete =
            || ReadFailure::rejected(FailureCode::ProviderRejected, Sentence::FreeBusyIncomplete);
        let Some(calendars) = answer.get("calendars").and_then(Value::as_object) else {
            return Err(ReadFailure::transient());
        };
        let mut busy = Vec::new();
        for target in targets {
            let Some(entry) = calendars
                .get(target.provider_id.as_str())
                .and_then(Value::as_object)
            else {
                return Err(incomplete());
            };
            match entry.get("errors") {
                None | Some(Value::Null) => {}
                Some(Value::Array(errors)) if errors.is_empty() => {}
                Some(_) => return Err(incomplete()),
            }
            let periods = match entry.get("busy") {
                None | Some(Value::Null) => &[][..],
                Some(Value::Array(periods)) => periods.as_slice(),
                Some(_) => return Err(ReadFailure::transient()),
            };
            let mut cut: Vec<(i64, i64)> = Vec::new();
            for period in periods {
                let instant = |name: &str| {
                    period
                        .get(name)
                        .and_then(Value::as_str)
                        .and_then(|t| parse_rfc3339_ms(t).ok())
                };
                let (Some(start), Some(end)) = (instant("start"), instant("end")) else {
                    return Err(ReadFailure::transient());
                };
                let (start, end) = (start.max(window_start), end.min(window_end));
                if start < end {
                    cut.push((start, end));
                }
            }
            cut.sort_unstable();
            for (start, end) in cut {
                let (Ok(start), Ok(end)) = (format_timestamp(start), format_timestamp(end)) else {
                    return Err(ReadFailure::transient());
                };
                busy.push(BusyInterval {
                    calendar_id: target.calendar.calendar_id.clone(),
                    start,
                    end,
                });
            }
        }
        let busy = AnyList::new(busy).map_err(|_| ReadFailure::transient())?;
        Ok(ToolResult::Availability { observed_at, busy })
    }
}
