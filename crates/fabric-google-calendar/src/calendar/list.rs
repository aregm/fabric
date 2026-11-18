//! The calendar list (6.2).

use fabric_desktop_contracts::common::FailureCode;
use fabric_desktop_contracts::scalar::Id;
use serde_json::Value;

use super::{CalendarListEntry, Google, ReadAnswer};
use crate::http::{BodyBound, Method, Request, Target};
use crate::outcome::{ReadFailure, ReadKind, Sentence};
use crate::translate::calendar_list_item;

/// More calendars than this fail the listing (6.2, PD23, ST2).
pub const MAX_CALENDARS: usize = 250;

/// The most pages one listing reads (6.2, 18.6). A listing that needs more fails as a whole,
/// as a list past 250 calendars does: Google may fill a page with fewer entries than asked
/// for, and a cut list would look like lost calendars (CO7).
pub const MAX_LIST_PAGES: usize = 20;

impl Google {
    /// `GET /users/me/calendarList?maxResults=250&minAccessRole=freeBusyReader&showHidden=true`,
    /// page after page (6.2). More than 250 calendars in all, or more than 20 pages, fails the
    /// listing (PD23, 18.6): a cut list would look like lost calendars (CO7). Entries with an
    /// unusable id, an unknown role or an unusable zone are left out; they count toward the
    /// 250.
    pub fn list_calendars(
        &mut self,
        account: &Id,
        deadline_ms: u64,
    ) -> Result<Vec<CalendarListEntry>, ReadFailure> {
        let token = self.read_token(account, deadline_ms)?;
        let mut entries = Vec::new();
        let mut seen = 0usize;
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let mut request = Request::new(Method::Get, Target::CalendarList, BodyBound::Large)
                .query("maxResults", "250")
                .query("minAccessRole", "freeBusyReader")
                .query("showHidden", "true");
            if let Some(next) = page_token.take() {
                request = request.query("pageToken", next);
            }
            let ReadAnswer::Body(body) =
                self.send_read(account, &token, request, deadline_ms, ReadKind::Calendar)?
            else {
                // `ReadKind::Calendar` never answers `Absent`.
                return Err(ReadFailure::transient());
            };
            let items = match body.get("items") {
                None | Some(Value::Null) => &[][..],
                Some(Value::Array(items)) => items.as_slice(),
                Some(_) => return Err(ReadFailure::transient()),
            };
            seen += items.len();
            if seen > MAX_CALENDARS {
                return Err(too_many());
            }
            for item in items {
                let Ok(item) = calendar_list_item(item) else {
                    return Err(ReadFailure::transient());
                };
                if let Some(item) = item {
                    entries.push(CalendarListEntry {
                        provider_id: item.id,
                        name: item.name,
                        primary: item.primary,
                        time_zone: item.time_zone,
                        access: item.access,
                    });
                }
            }
            match body.get("nextPageToken") {
                None | Some(Value::Null) => return Ok(entries),
                Some(Value::String(next)) => page_token = Some(next.clone()),
                Some(_) => return Err(ReadFailure::transient()),
            }
        }
        Err(too_many())
    }
}

fn too_many() -> ReadFailure {
    ReadFailure::rejected(FailureCode::ProviderRejected, Sentence::TooManyCalendars)
}
