//! The request log (14.1): every request the fake received, in order.

use serde_json::Value;

/// One request as the fake received it, and what the fake did with it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoggedRequest {
    /// Its place in the log, from 0.
    pub seq: usize,
    /// The number of the connection it came on, from 1 (one request per connection).
    pub connection: u64,
    /// The method as sent.
    pub method: String,
    /// The request target as sent (path and query, still encoded).
    pub target: String,
    /// The path as sent, still encoded.
    pub raw_path: String,
    /// The path with each segment decoded, joined by `/`. A decoded `/` inside a segment stays
    /// a `/`, so compare [`LoggedRequest::segments`] where that matters.
    pub path: String,
    /// The decoded path segments (`/calendar/v3/calendars/a%40b/events` gives `calendar`, `v3`,
    /// `calendars`, `a@b`, `events`).
    pub segments: Vec<String>,
    /// The decoded query pairs, in order, duplicates kept.
    pub query: Vec<(String, String)>,
    /// The headers, in order, names as sent.
    pub headers: Vec<(String, String)>,
    /// Whether an `Authorization` header was present.
    pub authorization: bool,
    /// The subject of the account whose token the request carried (a bearer token for the
    /// Calendar API, the code or refresh token for the token endpoint, the token for
    /// revocation), when the fake knows the token, valid or not.
    pub token_account: Option<String>,
    /// The `If-Match` header.
    pub if_match: Option<String>,
    /// The `Content-Type` header.
    pub content_type: Option<String>,
    /// The `Accept-Encoding` header.
    pub accept_encoding: Option<String>,
    /// The body as received.
    pub body: Vec<u8>,
    /// The decoded form fields of a form-encoded body (the token and revocation endpoints).
    pub form: Option<Vec<(String, String)>>,
    /// Why the fake could not read the request, when it could not (a bound, a broken head).
    pub problem: Option<String>,
    /// The labels of the faults that fired on this request.
    pub faults: Vec<String>,
    /// The status the fake answered; `None` when it closed without an answer, or has not
    /// answered yet.
    pub answered: Option<u16>,
}

impl LoggedRequest {
    /// The first value of a query parameter.
    pub fn query_value(&self, name: &str) -> Option<&str> {
        first(&self.query, name)
    }

    /// Every value of a query parameter.
    pub fn query_values(&self, name: &str) -> Vec<&str> {
        all(&self.query, name)
    }

    /// The first header of this name, without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The first value of a form field.
    pub fn form_value(&self, name: &str) -> Option<&str> {
        self.form.as_deref().and_then(|f| first(f, name))
    }

    /// The body read as JSON, when it is JSON.
    pub fn body_json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// The body as text (invalid UTF-8 replaced).
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Whether the method can change Google: POST, PATCH, PUT or DELETE (PD13).
    pub fn is_changing(&self) -> bool {
        matches!(self.method.as_str(), "POST" | "PATCH" | "PUT" | "DELETE")
    }

    /// Whether this is a Calendar API request (`/calendar/v3/...`).
    pub fn is_calendar(&self) -> bool {
        self.path.starts_with("/calendar/v3/")
    }

    /// Whether this request can change a calendar: a changing method on the Calendar API
    /// other than the free/busy query (which is a POST that reads).
    pub fn is_calendar_change(&self) -> bool {
        self.is_calendar() && self.is_changing() && self.path != "/calendar/v3/freeBusy"
    }
}

fn first<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn all<'a>(pairs: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    pairs
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// A copy of the log, or of a part of it. The filters return a new `Log` so that they chain:
/// `fake.log().method("PATCH").path_prefix("/calendar/v3/").len()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    entries: Vec<LoggedRequest>,
}

impl Log {
    pub(crate) fn new(entries: Vec<LoggedRequest>) -> Log {
        Log { entries }
    }

    /// The entries, in the order received.
    pub fn entries(&self) -> &[LoggedRequest] {
        &self.entries
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries in order.
    pub fn iter(&self) -> std::slice::Iter<'_, LoggedRequest> {
        self.entries.iter()
    }

    /// The last entry.
    pub fn last(&self) -> Option<&LoggedRequest> {
        self.entries.last()
    }

    /// The entries that satisfy `keep`.
    pub fn filter(&self, keep: impl Fn(&LoggedRequest) -> bool) -> Log {
        Log::new(self.entries.iter().filter(|e| keep(e)).cloned().collect())
    }

    /// The entries with this method.
    pub fn method(&self, method: &str) -> Log {
        self.filter(|e| e.method == method)
    }

    /// The entries whose decoded path starts with `prefix`.
    pub fn path_prefix(&self, prefix: &str) -> Log {
        self.filter(|e| e.path.starts_with(prefix))
    }

    /// The entries whose decoded path is exactly `path`.
    pub fn path(&self, path: &str) -> Log {
        self.filter(|e| e.path == path)
    }

    /// The POST, PATCH, PUT and DELETE requests, wherever they went.
    pub fn changing(&self) -> Log {
        self.filter(LoggedRequest::is_changing)
    }

    /// The requests that can change a calendar (PD13, g40).
    pub fn calendar_changes(&self) -> Log {
        self.filter(LoggedRequest::is_calendar_change)
    }

    /// The Calendar API requests.
    pub fn calendar(&self) -> Log {
        self.filter(LoggedRequest::is_calendar)
    }

    /// The requests to the token endpoint.
    pub fn token_requests(&self) -> Log {
        self.path("/token")
    }

    /// The requests to the revocation endpoint.
    pub fn revocations(&self) -> Log {
        self.path("/revoke")
    }

    /// The requests for the key set.
    pub fn key_set_fetches(&self) -> Log {
        self.path("/oauth2/v3/certs")
    }

    /// The requests to the authorization endpoint (made by the browser).
    pub fn authorizations(&self) -> Log {
        self.path("/o/oauth2/v2/auth")
    }

    /// Whether any request's raw path or decoded path segments contain `needle` (for "never
    /// `primary`", PD22).
    pub fn any_path_contains(&self, needle: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.raw_path.contains(needle) || e.segments.iter().any(|s| s.contains(needle)))
    }
}

impl<'a> IntoIterator for &'a Log {
    type Item = &'a LoggedRequest;
    type IntoIter = std::slice::Iter<'a, LoggedRequest>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}
