//! The Calendar API side of the fake (6.2 to 6.8): `calendarList.list`, `freeBusy.query`, and
//! events list, get, insert, patch and delete, under `/calendar/v3`, answered as G-DOC says.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};

use crate::faults::Plan;
use crate::server::{Request, Response, media_type, message_for};
use crate::state::{CalendarEntry, DeletedEventGet, State, StoredEvent, merge_patch};
use crate::time::{format_utc, format_utc_millis, parse_date, parse_rfc3339};

const AUTH: &str = "https://www.googleapis.com/auth/";

/// The scopes Google accepts for each kind of call (G-DOC; LV12 checks them live).
const LIST_SCOPES: [&str; 4] = [
    "calendar",
    "calendar.readonly",
    "calendar.calendarlist",
    "calendar.calendarlist.readonly",
];
const FREEBUSY_SCOPES: [&str; 4] = [
    "calendar",
    "calendar.readonly",
    "calendar.freebusy",
    "calendar.events.freebusy",
];
const READ_SCOPES: [&str; 6] = [
    "calendar",
    "calendar.readonly",
    "calendar.events",
    "calendar.events.readonly",
    "calendar.events.owned",
    "calendar.events.owned.readonly",
];
const WRITE_SCOPES: [&str; 3] = ["calendar", "calendar.events", "calendar.events.owned"];

enum Op {
    CalendarList,
    FreeBusy,
    List(String),
    Insert(String),
    Get(String, String),
    Patch(String, String),
    Delete(String, String),
}

fn error(status: u16, reason: &str, message: &str) -> Response {
    Response::google_error(status, reason, message)
}

fn not_found() -> Response {
    error(404, "notFound", &message_for("notFound"))
}

fn invalid(message: &str) -> Response {
    error(400, "invalid", message)
}

pub(crate) fn handle(state: &mut State, request: &Request, plan: &Plan) -> Response {
    let rest: Vec<&str> = request.segments[2..].iter().map(String::as_str).collect();
    let op = match (request.method.as_str(), rest.as_slice()) {
        ("GET", ["users", "me", "calendarList"]) => Op::CalendarList,
        ("POST", ["freeBusy"]) => Op::FreeBusy,
        ("GET", ["calendars", cal, "events"]) => Op::List(cal.to_string()),
        ("POST", ["calendars", cal, "events"]) => Op::Insert(cal.to_string()),
        ("GET", ["calendars", cal, "events", id]) => Op::Get(cal.to_string(), id.to_string()),
        ("PATCH", ["calendars", cal, "events", id]) => Op::Patch(cal.to_string(), id.to_string()),
        ("DELETE", ["calendars", cal, "events", id]) => Op::Delete(cal.to_string(), id.to_string()),
        _ => return not_found(),
    };
    let (subject, scopes) = match authenticate(state, request, plan) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let needed: &[&str] = match op {
        Op::CalendarList => &LIST_SCOPES,
        Op::FreeBusy => &FREEBUSY_SCOPES,
        Op::List(_) | Op::Get(..) => &READ_SCOPES,
        Op::Insert(_) | Op::Patch(..) | Op::Delete(..) => &WRITE_SCOPES,
    };
    let allowed = scopes.iter().any(|s| {
        s.strip_prefix(AUTH)
            .is_some_and(|short| needed.contains(&short))
    });
    if !allowed {
        return error(
            403,
            "insufficientPermissions",
            &message_for("insufficientPermissions"),
        );
    }
    match op {
        Op::CalendarList => calendar_list(state, &subject, request),
        Op::FreeBusy => free_busy(state, &subject, request),
        Op::List(cal) => list_events(state, &subject, &cal, request, plan),
        Op::Insert(cal) => insert(state, &subject, &cal, request, plan),
        Op::Get(cal, id) => get(state, &subject, &cal, &id, request, plan),
        Op::Patch(cal, id) => patch(state, &subject, &cal, &id, request, plan),
        Op::Delete(cal, id) => delete(state, &subject, &cal, &id, request),
    }
}

/// The bearer token must belong to an account and be unexpired (6.10: 401 otherwise).
fn authenticate(
    state: &mut State,
    request: &Request,
    plan: &Plan,
) -> Result<(String, Vec<String>), Response> {
    let Some(value) = request.header("authorization") else {
        return Err(error(401, "required", "Login Required."));
    };
    let Some(token) = value.strip_prefix("Bearer ").map(str::trim) else {
        return Err(error(401, "authError", &message_for("authError")));
    };
    let now = state.now_ms;
    let Some(found) = state.access_tokens.iter_mut().find(|t| t.token == token) else {
        return Err(error(401, "authError", &message_for("authError")));
    };
    if plan.expired_token {
        found.ends_ms = found.ends_ms.min(now);
    }
    if found.revoked || found.ends_ms <= now {
        return Err(error(401, "authError", &message_for("authError")));
    }
    let (subject, scopes) = (found.subject.clone(), found.scopes.clone());
    if state.account(&subject).is_none() {
        return Err(error(401, "authError", &message_for("authError")));
    }
    Ok((subject, scopes))
}

/// The account's list entry for a calendar id (`primary` names the primary calendar, as at
/// Google). A deleted entry or a calendar the account does not list is not found.
fn resolve(state: &State, subject: &str, calendar: &str) -> Option<CalendarEntry> {
    let account = state.account(subject)?;
    account
        .calendars
        .iter()
        .find(|c| {
            !c.deleted
                && if calendar == "primary" {
                    c.primary
                } else {
                    c.id == calendar
                }
        })
        .cloned()
}

fn role_rank(role: &str) -> Option<u8> {
    match role {
        "freeBusyReader" => Some(1),
        "reader" => Some(2),
        "writerWithoutPrivateAccess" => Some(3),
        "writer" => Some(4),
        "owner" => Some(5),
        _ => None,
    }
}

fn can_read_events(role: &str) -> bool {
    // An unknown role reads like a reader; it never writes.
    role_rank(role).is_none_or(|r| r >= 2)
}

fn can_write(role: &str) -> bool {
    role_rank(role).is_some_and(|r| r >= 3)
}

fn bool_param(request: &Request, name: &str) -> Result<Option<bool>, Response> {
    match request.query_all(name).first() {
        None => Ok(None),
        Some(&"true") => Ok(Some(true)),
        Some(&"false") => Ok(Some(false)),
        Some(_) => Err(invalid(&format!("Invalid boolean value for: {name}"))),
    }
}

fn number_param(request: &Request, name: &str) -> Result<Option<i64>, Response> {
    match request.query_all(name).first() {
        None => Ok(None),
        Some(v) => match v.parse::<i64>() {
            Ok(n) => Ok(Some(n)),
            Err(_) => Err(invalid(&format!("Invalid value for: {name}"))),
        },
    }
}

fn time_param(request: &Request, name: &str) -> Result<Option<i64>, Response> {
    match request.query_all(name).first() {
        None => Ok(None),
        Some(v) => match parse_rfc3339(v) {
            Some(ms) => Ok(Some(ms)),
            None => Err(invalid(&format!("Bad Request: invalid {name}"))),
        },
    }
}

fn page_token(offset: usize) -> String {
    URL_SAFE_NO_PAD.encode(format!("fake-page:{offset}"))
}

fn read_page_token(request: &Request) -> Result<usize, Response> {
    let Some(token) = request.query_all("pageToken").first().copied() else {
        return Ok(0);
    };
    URL_SAFE_NO_PAD
        .decode(token)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.strip_prefix("fake-page:").and_then(|n| n.parse().ok()))
        .ok_or_else(|| invalid("Invalid page token value."))
}

fn send_updates_ok(request: &Request) -> Result<(), Response> {
    match request.query_all("sendUpdates").first() {
        None | Some(&"all") | Some(&"externalOnly") | Some(&"none") => Ok(()),
        Some(_) => Err(invalid("Invalid value for: sendUpdates")),
    }
}

/// `GET /users/me/calendarList` (6.2).
fn calendar_list(state: &State, subject: &str, request: &Request) -> Response {
    let max = match number_param(request, "maxResults") {
        Ok(Some(n)) if n < 1 => return invalid("Invalid value for: maxResults"),
        Ok(n) => (n.unwrap_or(100).min(250) as usize)
            .min(state.calendar_list_page_size.unwrap_or(usize::MAX)),
        Err(r) => return r,
    };
    let min_rank = match request.query_all("minAccessRole").first() {
        None => 0,
        Some(role) => match role_rank(role) {
            Some(r) if *role != "writerWithoutPrivateAccess" => r,
            _ => return invalid("Invalid value for: minAccessRole"),
        },
    };
    let show_hidden = match bool_param(request, "showHidden") {
        Ok(v) => v.unwrap_or(false),
        Err(r) => return r,
    };
    let show_deleted = match bool_param(request, "showDeleted") {
        Ok(v) => v.unwrap_or(false),
        Err(r) => return r,
    };
    let offset = match read_page_token(request) {
        Ok(o) => o,
        Err(r) => return r,
    };
    let Some(account) = state.account(subject) else {
        return not_found();
    };
    let entries: Vec<(usize, &CalendarEntry)> = account
        .calendars
        .iter()
        .enumerate()
        .filter(|(_, c)| show_deleted || !c.deleted)
        .filter(|(_, c)| show_hidden || !c.hidden)
        // An unknown role passes every filter, so that the adapter sees it (g18).
        .filter(|(_, c)| role_rank(&c.access_role).is_none_or(|r| r >= min_rank))
        .collect();
    let page: Vec<Value> = entries
        .iter()
        .skip(offset)
        .take(max)
        .map(|(index, c)| {
            let mut item = Map::new();
            item.insert("kind".into(), json!("calendar#calendarListEntry"));
            item.insert(
                "etag".into(),
                json!(format!("\"{}\"", 1_700_000_000_000 + *index as u64)),
            );
            item.insert("id".into(), json!(c.id));
            if let Some(s) = &c.summary {
                item.insert("summary".into(), json!(s));
            }
            if let Some(s) = &c.summary_override {
                item.insert("summaryOverride".into(), json!(s));
            }
            item.insert("timeZone".into(), json!(c.time_zone));
            item.insert("colorId".into(), json!("1"));
            item.insert("backgroundColor".into(), json!("#ac725e"));
            item.insert("foregroundColor".into(), json!("#1d1d1d"));
            item.insert("accessRole".into(), json!(c.access_role));
            item.insert("defaultReminders".into(), json!([]));
            if c.primary {
                item.insert("primary".into(), json!(true));
            }
            if c.hidden {
                item.insert("hidden".into(), json!(true));
            }
            if c.deleted {
                item.insert("deleted".into(), json!(true));
            }
            Value::Object(item)
        })
        .collect();
    let mut body = json!({
        "kind": "calendar#calendarList",
        "etag": "\"p33fake\"",
        "items": page,
    });
    if offset + max < entries.len() {
        body["nextPageToken"] = json!(page_token(offset + max));
    }
    Response::json(200, &body)
}

fn json_body(request: &Request) -> Result<Map<String, Value>, Response> {
    let is_json = request
        .header("content-type")
        .is_some_and(|ct| media_type(ct).eq_ignore_ascii_case("application/json"));
    if !is_json {
        return Err(error(400, "parseError", "Parse Error"));
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(error(400, "parseError", "Parse Error")),
    }
}

/// `POST /freeBusy` (6.3).
fn free_busy(state: &State, subject: &str, request: &Request) -> Response {
    let body = match json_body(request) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let time = |name: &str| -> Result<(String, i64), Response> {
        let Some(text) = body.get(name).and_then(Value::as_str) else {
            return Err(error(400, "required", &format!("Missing {name}.")));
        };
        parse_rfc3339(text)
            .map(|ms| (text.to_string(), ms))
            .ok_or_else(|| invalid(&format!("Invalid {name}.")))
    };
    let (min_text, min) = match time("timeMin") {
        Ok(t) => t,
        Err(r) => return r,
    };
    let (max_text, max) = match time("timeMax") {
        Ok(t) => t,
        Err(r) => return r,
    };
    if max <= min {
        return error(400, "timeRangeEmpty", "The specified time range is empty.");
    }
    let Some(items) = body.get("items").and_then(Value::as_array) else {
        return error(400, "required", "Missing items.");
    };
    if items.len() > 50 {
        return error(
            400,
            "tooManyCalendarsRequested",
            "Too many calendars requested.",
        );
    }
    let mut calendars = Map::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            return error(400, "required", "Missing id.");
        };
        let answer = match resolve(state, subject, id) {
            None => json!({"errors": [{"domain": "global", "reason": "notFound"}], "busy": []}),
            Some(entry) => json!({"busy": busy_periods(state, &entry.id, min, max)}),
        };
        calendars.insert(id.to_string(), answer);
    }
    Response::json(
        200,
        &json!({
            "kind": "calendar#freeBusy",
            "timeMin": min_text,
            "timeMax": max_text,
            "calendars": calendars,
        }),
    )
}

/// Busy periods of a calendar: its timed events that are not cancelled, not transparent, not
/// a recurring master (occurrences are stored as events), not hidden, and overlap the window.
/// They are not cut to the window (the adapter cuts them, 6.3).
fn busy_periods(state: &State, calendar: &str, min: i64, max: i64) -> Vec<Value> {
    let Some(cal) = state.calendars.get(calendar) else {
        return Vec::new();
    };
    let mut periods: Vec<(i64, i64)> = cal
        .events
        .iter()
        .filter(|e| !e.cancelled() && e.hidden_reads == 0 && !is_master(&e.json))
        .filter(|e| e.json.get("transparency").and_then(Value::as_str) != Some("transparent"))
        .filter_map(|e| {
            let start = e.json.get("start")?.get("dateTime")?.as_str()?;
            let end = e.json.get("end")?.get("dateTime")?.as_str()?;
            Some((parse_rfc3339(start)?, parse_rfc3339(end)?))
        })
        .filter(|(s, e)| *e > min && *s < max)
        .collect();
    periods.sort();
    if state.free_busy_merge {
        let mut merged: Vec<(i64, i64)> = Vec::new();
        for (s, e) in periods {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        periods = merged;
    }
    periods
        .into_iter()
        .map(|(s, e)| json!({"start": format_utc(s), "end": format_utc(e)}))
        .collect()
}

fn is_master(event: &Map<String, Value>) -> bool {
    event
        .get("recurrence")
        .and_then(Value::as_array)
        .is_some_and(|r| !r.is_empty())
}

/// The instant of `start` or `end`: its `dateTime`, or its `date` as midnight UTC.
fn instant(event: &Map<String, Value>, member: &str) -> Option<i64> {
    let time = event.get(member)?;
    if let Some(dt) = time.get("dateTime").and_then(Value::as_str) {
        return parse_rfc3339(dt);
    }
    time.get("date")
        .and_then(Value::as_str)
        .and_then(parse_date)
}

/// An event as an answer shows it to an account with `role`: a private event without its
/// details for a reader, attendees cut by `maxAttendees` or a fault.
fn render(
    event: &Map<String, Value>,
    role: &str,
    max_attendees: Option<usize>,
    cut: bool,
) -> Value {
    let mut out = event.clone();
    let private = out.get("visibility").and_then(Value::as_str) == Some("private");
    if private && matches!(role, "reader" | "writerWithoutPrivateAccess") {
        const KEPT: [&str; 14] = [
            "kind",
            "etag",
            "id",
            "status",
            "start",
            "end",
            "recurringEventId",
            "originalStartTime",
            "visibility",
            "transparency",
            "iCalUID",
            "sequence",
            "created",
            "updated",
        ];
        out.retain(|k, _| KEPT.contains(&k.as_str()));
    }
    let count = out
        .get("attendees")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if cut || max_attendees.is_some_and(|max| count > max) {
        let own: Vec<Value> = out
            .get("attendees")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter(|a| a.get("self").and_then(Value::as_bool) == Some(true))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if own.is_empty() {
            out.remove("attendees");
        } else {
            out.insert("attendees".into(), Value::Array(own));
        }
        out.insert("attendeesOmitted".into(), json!(true));
    }
    Value::Object(out)
}

fn max_attendees(request: &Request) -> Result<Option<usize>, Response> {
    match number_param(request, "maxAttendees")? {
        None => Ok(None),
        Some(n) if n >= 1 => Ok(Some(n as usize)),
        Some(_) => Err(invalid("Invalid value for: maxAttendees")),
    }
}

/// `GET /calendars/<id>/events` (6.4). `fields` and `timeZone` are logged and otherwise
/// ignored: every member is sent, and every `dateTime` as stored.
fn list_events(
    state: &State,
    subject: &str,
    calendar: &str,
    request: &Request,
    plan: &Plan,
) -> Response {
    let Some(entry) = resolve(state, subject, calendar) else {
        return not_found();
    };
    if !can_read_events(&entry.access_role) {
        return error(403, "forbidden", &message_for("forbidden"));
    }
    let parsed = (|| {
        let min = time_param(request, "timeMin")?;
        let max = time_param(request, "timeMax")?;
        let single = bool_param(request, "singleEvents")?.unwrap_or(false);
        let show_deleted = bool_param(request, "showDeleted")?.unwrap_or(false);
        let page_size = match number_param(request, "maxResults")? {
            Some(n) if n < 1 => return Err(invalid("Invalid value for: maxResults")),
            Some(n) => (n as usize).min(2500),
            None => 250,
        };
        let order = request.query_all("orderBy").first().map(|s| s.to_string());
        match order.as_deref() {
            None | Some("updated") => {}
            Some("startTime") if single => {}
            Some("startTime") => {
                return Err(error(
                    400,
                    "badRequest",
                    "The requested ordering is not available for the particular query.",
                ));
            }
            Some(_) => return Err(invalid("Invalid value for: orderBy")),
        }
        if let (Some(a), Some(b)) = (min, max)
            && b <= a
        {
            return Err(error(
                400,
                "timeRangeEmpty",
                "The specified time range is empty.",
            ));
        }
        let offset = read_page_token(request)?;
        let attendees = max_attendees(request)?;
        Ok((
            min,
            max,
            single,
            show_deleted,
            page_size,
            order,
            offset,
            attendees,
        ))
    })();
    let (min, max, single, show_deleted, page_size, order, offset, attendees) = match parsed {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(cal) = state.calendars.get(&entry.id) else {
        return not_found();
    };
    let mut selected: Vec<&StoredEvent> = cal
        .events
        .iter()
        .filter(|e| e.hidden_reads == 0)
        .filter(|e| show_deleted || !e.cancelled())
        .filter(|e| !single || !is_master(&e.json))
        .filter(|e| {
            let start = instant(&e.json, "start");
            let end = instant(&e.json, "end").or(start);
            min.is_none_or(|m| end.is_some_and(|x| x > m))
                && max.is_none_or(|m| start.is_some_and(|x| x < m))
        })
        .collect();
    match order.as_deref() {
        Some("startTime") => {
            selected.sort_by_key(|e| (instant(&e.json, "start"), e.id().to_string()))
        }
        Some("updated") => selected.sort_by_key(|e| {
            e.json
                .get("updated")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339)
        }),
        _ => {}
    }
    let items: Vec<Value> = selected
        .iter()
        .skip(offset)
        .take(page_size)
        .map(|e| render(&e.json, &entry.access_role, attendees, plan.cut_attendees))
        .collect();
    let mut body = json!({
        "kind": "calendar#events",
        "etag": format!("\"{}\"", state.revision),
        "summary": entry.summary_override.clone().or(entry.summary.clone()).unwrap_or_default(),
        "description": "",
        "updated": format_utc_millis(state.now_ms),
        "timeZone": cal.time_zone,
        "accessRole": entry.access_role,
        "defaultReminders": [],
        "items": items,
    });
    if offset + page_size < selected.len() {
        body["nextPageToken"] = json!(page_token(offset + page_size));
    }
    Response::json(200, &body)
}

/// `GET /calendars/<id>/events/<event id>` (6.5).
fn get(
    state: &mut State,
    subject: &str,
    calendar: &str,
    id: &str,
    request: &Request,
    plan: &Plan,
) -> Response {
    let Some(entry) = resolve(state, subject, calendar) else {
        return not_found();
    };
    if !can_read_events(&entry.access_role) {
        return error(403, "forbidden", &message_for("forbidden"));
    }
    let attendees = match max_attendees(request) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let mode = state.deleted_event_get;
    let Some(event) = state.stored_mut(&entry.id, id) else {
        return not_found();
    };
    if event.hidden_reads > 0 {
        event.hidden_reads -= 1;
        return not_found();
    }
    if event.cancelled() {
        match mode {
            DeletedEventGet::NotFound => return not_found(),
            DeletedEventGet::Gone => return error(410, "deleted", &message_for("deleted")),
            DeletedEventGet::Cancelled => {}
        }
    }
    Response::json(
        200,
        &render(
            &event.json,
            &entry.access_role,
            attendees,
            plan.cut_attendees,
        ),
    )
}

/// Google's rule for a client-chosen id: 5 to 1024 characters of `a-v` and `0-9`.
fn valid_client_id(id: &str) -> bool {
    (5..=1024).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'a'..=b'v' | b'0'..=b'9'))
}

/// Members a client cannot set; dropped from an insert or patch body.
const READ_ONLY: [&str; 6] = [
    "kind",
    "etag",
    "created",
    "updated",
    "htmlLink",
    "attendeesOmitted",
];

/// Checks `start` and `end`: both present, each with exactly one of `dateTime` and `date`, of
/// the same kind, readable, and the end not before the start.
fn check_times(event: &Map<String, Value>) -> Result<(), Response> {
    let kind = |member: &str| -> Result<(bool, i64), Response> {
        let Some(time) = event.get(member).and_then(Value::as_object) else {
            return Err(error(400, "required", &format!("Missing {member} time.")));
        };
        let date_time = time.get("dateTime").and_then(Value::as_str);
        let date = time.get("date").and_then(Value::as_str);
        match (date_time, date) {
            (Some(dt), None) => parse_rfc3339(dt)
                .map(|ms| (true, ms))
                .ok_or_else(|| invalid(&format!("Invalid {member} time."))),
            (None, Some(d)) => parse_date(d)
                .map(|ms| (false, ms))
                .ok_or_else(|| invalid(&format!("Invalid {member} time."))),
            _ => Err(invalid(&format!("Invalid {member} time."))),
        }
    };
    let (start_timed, start) = kind("start")?;
    let (end_timed, end) = kind("end")?;
    if start_timed != end_timed {
        return Err(invalid(
            "Start and end times must either both be date or both be dateTime.",
        ));
    }
    if end < start {
        return Err(error(
            400,
            "timeRangeEmpty",
            "The specified time range is empty.",
        ));
    }
    Ok(())
}

/// What Google adds to attendees written through the API: `self` on the entry whose address
/// is the calendar's id, and `responseStatus` `needsAction` where it is missing.
fn mark_attendees(event: &mut Map<String, Value>, calendar: &str) {
    if let Some(Value::Array(list)) = event.get_mut("attendees") {
        for attendee in list.iter_mut().filter_map(Value::as_object_mut) {
            let own = attendee
                .get("email")
                .and_then(Value::as_str)
                .is_some_and(|e| e.eq_ignore_ascii_case(calendar));
            if own && !attendee.contains_key("self") {
                attendee.insert("self".into(), json!(true));
            }
            attendee
                .entry("responseStatus")
                .or_insert_with(|| json!("needsAction"));
        }
    }
}

/// `POST /calendars/<id>/events` (6.6).
fn insert(
    state: &mut State,
    subject: &str,
    calendar: &str,
    request: &Request,
    plan: &Plan,
) -> Response {
    let Some(entry) = resolve(state, subject, calendar) else {
        return not_found();
    };
    if !can_write(&entry.access_role) {
        return error(
            403,
            "requiredAccessLevel",
            "You need to have writer access to this calendar.",
        );
    }
    if let Err(r) = send_updates_ok(request) {
        return r;
    }
    let mut event = match json_body(request) {
        Ok(b) => b,
        Err(r) => return r,
    };
    for member in READ_ONLY {
        event.remove(member);
    }
    if let Err(r) = check_times(&event) {
        return r;
    }
    let id = match event.get("id") {
        None => state.new_event_id(),
        Some(Value::String(id)) if valid_client_id(id) => id.clone(),
        Some(_) => return invalid("Invalid resource id value."),
    };
    if state.stored(&entry.id, &id).is_some() {
        return error(409, "duplicate", &message_for("duplicate"));
    }
    let account_email = state
        .account(subject)
        .map(|a| a.email.clone())
        .unwrap_or_default();
    let now = format_utc_millis(state.now_ms);
    event.insert("id".into(), json!(id));
    event.insert("kind".into(), json!("calendar#event"));
    event.insert("etag".into(), json!(state.next_etag()));
    event.insert("created".into(), json!(now));
    event.insert("updated".into(), json!(now));
    event.entry("status").or_insert_with(|| json!("confirmed"));
    event.entry("eventType").or_insert_with(|| json!("default"));
    event
        .entry("iCalUID")
        .or_insert_with(|| json!(format!("{id}@google.com")));
    event.entry("sequence").or_insert_with(|| json!(0));
    event.entry("creator").or_insert_with(|| {
        let mut creator = json!({"email": account_email});
        if account_email.eq_ignore_ascii_case(&entry.id) {
            creator["self"] = json!(true);
        }
        creator
    });
    event
        .entry("organizer")
        .or_insert_with(|| json!({"email": entry.id, "self": true}));
    mark_attendees(&mut event, &entry.id);
    let rendered = render(&event, &entry.access_role, None, plan.cut_attendees);
    state
        .calendar_or_create(&entry.id)
        .events
        .push(StoredEvent {
            json: event,
            hidden_reads: plan.hide_insert_reads.unwrap_or(0),
        });
    Response::json(200, &rendered)
}

/// Whether `If-Match` lets a change through: absent, `*`, or a list holding the event's etag
/// as a strong tag (FA1).
fn if_match_ok(header: Option<&str>, etag: &str) -> bool {
    let Some(value) = header else {
        return true;
    };
    let value = value.trim();
    value == "*"
        || value
            .split(',')
            .map(str::trim)
            .any(|tag| !tag.starts_with("W/") && tag == etag)
}

/// Finds the event a PATCH or DELETE names, with Google's answers for a missing, hidden or
/// cancelled one and for `If-Match`.
fn changeable<'a>(
    state: &'a mut State,
    calendar: &str,
    id: &str,
    request: &Request,
    patching: bool,
) -> Result<&'a mut StoredEvent, Response> {
    let mode = state.deleted_event_get;
    let Some(event) = state.stored_mut(calendar, id) else {
        return Err(not_found());
    };
    if event.hidden_reads > 0 {
        return Err(not_found());
    }
    if event.cancelled() {
        return Err(if patching && mode == DeletedEventGet::NotFound {
            not_found()
        } else {
            error(410, "deleted", &message_for("deleted"))
        });
    }
    let etag = event.json.get("etag").and_then(Value::as_str).unwrap_or("");
    if !if_match_ok(request.header("if-match"), etag) {
        return Err(error(
            412,
            "conditionNotMet",
            &message_for("conditionNotMet"),
        ));
    }
    Ok(event)
}

/// `PATCH /calendars/<id>/events/<event id>` (6.7): a JSON merge patch; with
/// `attendeesOmitted: true` in the body the given attendees are merged by address.
fn patch(
    state: &mut State,
    subject: &str,
    calendar: &str,
    id: &str,
    request: &Request,
    plan: &Plan,
) -> Response {
    let Some(entry) = resolve(state, subject, calendar) else {
        return not_found();
    };
    if !can_write(&entry.access_role) {
        return error(
            403,
            "requiredAccessLevel",
            "You need to have writer access to this calendar.",
        );
    }
    if let Err(r) = changeable(state, &entry.id, id, request, true) {
        return r;
    }
    if let Err(r) = send_updates_ok(request) {
        return r;
    }
    let mut body = match json_body(request) {
        Ok(b) => b,
        Err(r) => return r,
    };
    if let Some(given) = body.get("id")
        && given.as_str() != Some(id)
    {
        return invalid("Invalid resource id value.");
    }
    let by_address = body.get("attendeesOmitted").and_then(Value::as_bool) == Some(true);
    for member in READ_ONLY {
        body.remove(member);
    }
    let etag = state.next_etag();
    let now = format_utc_millis(state.now_ms);
    let Some(stored) = state.stored_mut(&entry.id, id) else {
        return not_found();
    };
    let mut changed = Value::Object(stored.json.clone());
    let attendees = if by_address {
        body.remove("attendees")
    } else {
        None
    };
    merge_patch(&mut changed, &Value::Object(body));
    let Value::Object(mut changed) = changed else {
        return invalid("Invalid event.");
    };
    if let Some(Value::Array(given)) = attendees {
        let mut list = changed
            .get("attendees")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for attendee in given {
            let address = attendee
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_string);
            let existing = list.iter_mut().find(|a| {
                address.as_deref().is_some_and(|addr| {
                    a.get("email")
                        .and_then(Value::as_str)
                        .is_some_and(|e| e.eq_ignore_ascii_case(addr))
                })
            });
            match existing {
                Some(slot) => merge_patch(slot, &attendee),
                None => list.push(attendee),
            }
        }
        changed.insert("attendees".into(), Value::Array(list));
    }
    if let Err(r) = check_times(&changed) {
        return r;
    }
    mark_attendees(&mut changed, &entry.id);
    changed.insert("etag".into(), json!(etag));
    changed.insert("updated".into(), json!(now));
    stored.json = changed;
    Response::json(
        200,
        &render(&stored.json, &entry.access_role, None, plan.cut_attendees),
    )
}

/// `DELETE /calendars/<id>/events/<event id>` (6.8): the event becomes `cancelled` (with its
/// stored occurrences when it is a recurring master) and the answer is 204 with no body.
fn delete(
    state: &mut State,
    subject: &str,
    calendar: &str,
    id: &str,
    request: &Request,
) -> Response {
    let Some(entry) = resolve(state, subject, calendar) else {
        return not_found();
    };
    if !can_write(&entry.access_role) {
        return error(
            403,
            "requiredAccessLevel",
            "You need to have writer access to this calendar.",
        );
    }
    if let Err(r) = changeable(state, &entry.id, id, request, false) {
        return r;
    }
    if let Err(r) = send_updates_ok(request) {
        return r;
    }
    let occurrences: Vec<String> = state
        .calendars
        .get(&entry.id)
        .map(|c| {
            c.events
                .iter()
                .filter(|e| {
                    !e.cancelled()
                        && e.json.get("recurringEventId").and_then(Value::as_str) == Some(id)
                })
                .map(|e| e.id().to_string())
                .collect()
        })
        .unwrap_or_default();
    state.cancel_event(&entry.id, id);
    for occurrence in occurrences {
        state.cancel_event(&entry.id, &occurrence);
    }
    Response::new(204)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_follow_googles_rule() {
        assert!(valid_client_id("abcde"));
        assert!(valid_client_id("0123456789abcdefghijklmnopqrstuv"));
        assert!(!valid_client_id("abcd"));
        assert!(!valid_client_id("abcdw"));
        assert!(!valid_client_id("ABCDE"));
        assert!(!valid_client_id(&"a".repeat(1025)));
    }

    #[test]
    fn if_match_takes_star_and_strong_tags_only() {
        assert!(if_match_ok(None, "\"1\""));
        assert!(if_match_ok(Some("*"), "\"1\""));
        assert!(if_match_ok(Some("\"1\""), "\"1\""));
        assert!(if_match_ok(Some("\"0\", \"1\""), "\"1\""));
        assert!(!if_match_ok(Some("W/\"1\""), "\"1\""));
        assert!(!if_match_ok(Some("\"2\""), "\"1\""));
        assert!(!if_match_ok(Some("1"), "\"1\""));
    }
}
