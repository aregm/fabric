//! The person at the browser (14.1): requests the authorization address the adapter built,
//! decides, and follows Google's redirect to the adapter's loopback listener with a plain HTTP
//! request, as a browser would.

use crate::FakeGoogle;
use crate::client::{ClientError, HttpResponse, RawRequest};
use crate::url::{HttpAddress, encode_pairs, parse_pairs};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Allow,
    Deny,
    CloseWindow,
}

/// What the person does on Google's consent screen, and how the callback is then sent.
///
/// Build it with [`Decision::allow`], [`Decision::deny`] or [`Decision::close_window`], then
/// narrow it: `Decision::allow().untick(&[SCOPE_EVENTS]).sign_in_as("sub-2")`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub(crate) choice: Choice,
    pub(crate) account: Option<String>,
    pub(crate) untick: Vec<String>,
    pub(crate) extra: Vec<String>,
    pub(crate) tamper: Vec<Tamper>,
}

impl Decision {
    fn of(choice: Choice) -> Decision {
        Decision {
            choice,
            account: None,
            untick: Vec::new(),
            extra: Vec::new(),
            tamper: Vec::new(),
        }
    }

    /// Allows everything that was asked. The account is the one [`Decision::sign_in_as`]
    /// names, else the one `login_hint` names, else the browser's account
    /// ([`crate::State::set_browser_account`]), else the first account added.
    pub fn allow() -> Decision {
        Decision::of(Choice::Allow)
    }

    /// Denies: the callback carries `error=access_denied`.
    pub fn deny() -> Decision {
        Decision::of(Choice::Deny)
    }

    /// Closes the window: no redirect at all, so the listener hears nothing.
    pub fn close_window() -> Decision {
        Decision::of(Choice::CloseWindow)
    }

    /// Unticks these scopes on the consent screen (FA5): the code carries the rest.
    pub fn untick(mut self, scopes: &[&str]) -> Decision {
        self.untick.extend(scopes.iter().map(|s| s.to_string()));
        self
    }

    /// Grants these scopes too, as when the person granted them to the same client earlier
    /// (3.6).
    pub fn grant_extra(mut self, scopes: &[&str]) -> Decision {
        self.extra.extend(scopes.iter().map(|s| s.to_string()));
        self
    }

    /// Signs in as this account (by subject), whatever `login_hint` said.
    pub fn sign_in_as(mut self, subject: &str) -> Decision {
        self.account = Some(subject.to_string());
        self
    }

    /// Changes the callback before it is sent (g07).
    pub fn tamper(mut self, tamper: Tamper) -> Decision {
        self.tamper.push(tamper);
        self
    }
}

/// A change to the callback request, for the listener's tests (g07).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tamper {
    /// `state` is this value instead.
    State(String),
    /// No `state`.
    DropState,
    /// The named parameter appears twice (its value repeated; for one the callback lacks, an
    /// empty value twice).
    DoubleParameter(String),
    /// Adds a parameter at the end.
    AddParameter(String, String),
    /// Removes every occurrence of a parameter.
    DropParameter(String),
    /// `iss` is this value instead.
    Iss(String),
    /// The request path is this instead of the redirect's.
    Path(String),
    /// The method is this instead of `GET`.
    Method(String),
    /// The `Host` header is this instead of `127.0.0.1:<port>`.
    Host(String),
}

/// Why the browser could not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserError {
    /// The address is not this fake's authorization endpoint; nothing was sent.
    NotTheAuthorizationEndpoint,
    /// The request to the fake failed.
    Client(ClientError),
}

/// What the authorization endpoint answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    /// Its status: 302 for a redirect, 200 for the consent page (window closed), 4xx for
    /// Google's error page.
    pub status: u16,
    /// The page's text.
    pub body: String,
    /// The redirect address, for a 302.
    pub location: Option<String>,
}

/// One whole visit: the authorization and, when there was a redirect, the listener's answer
/// to the callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    /// What the authorization endpoint answered.
    pub authorization: Authorization,
    /// The callback address as sent (after any [`Tamper`]), when there was a redirect.
    pub callback_address: Option<String>,
    /// What the listener answered, when there was a redirect.
    pub callback: Option<Result<HttpResponse, ClientError>>,
}

impl Visit {
    /// The `code` of the redirect, when Google issued one.
    pub fn code(&self) -> Option<String> {
        self.redirect_parameter("code")
    }

    /// A parameter of the redirect as Google sent it (before any [`Tamper`]).
    pub fn redirect_parameter(&self, name: &str) -> Option<String> {
        let location = self.authorization.location.as_deref()?;
        let address = HttpAddress::parse(location)?;
        parse_pairs(&address.query)?
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// The listener's answer, when the callback reached it.
    pub fn listener_answer(&self) -> Option<&HttpResponse> {
        self.callback.as_ref()?.as_ref().ok()
    }
}

/// The browser of one fake. Get it with [`FakeGoogle::browser`].
pub struct Browser<'a> {
    pub(crate) fake: &'a FakeGoogle,
}

impl Browser<'_> {
    /// Requests the authorization address with `decision` and returns what the fake
    /// answered, without following a redirect.
    pub fn authorize(
        &self,
        address: &str,
        decision: &Decision,
    ) -> Result<Authorization, BrowserError> {
        let parsed =
            HttpAddress::parse(address).ok_or(BrowserError::NotTheAuthorizationEndpoint)?;
        if parsed.host != "127.0.0.1"
            || parsed.port != self.fake.port()
            || parsed.path != "/o/oauth2/v2/auth"
        {
            return Err(BrowserError::NotTheAuthorizationEndpoint);
        }
        self.fake
            .with_state(|s| s.pending_decision = Some(decision.clone()));
        let sent = RawRequest::get(address)
            .map_err(BrowserError::Client)?
            .header("User-Agent", "FakeBrowser/1")
            .header("Accept", "text/html")
            .send();
        self.fake.with_state(|s| s.pending_decision = None);
        let response = sent.map_err(BrowserError::Client)?;
        Ok(Authorization {
            status: response.status,
            body: response.text(),
            location: response.header("location").map(str::to_string),
        })
    }

    /// Builds the callback for a redirect address with `tamper` applied: the address, the
    /// method and the `Host` it is sent with.
    pub fn callback_request(
        &self,
        location: &str,
        tamper: &[Tamper],
    ) -> Result<RawRequest, ClientError> {
        let address = HttpAddress::parse(location).ok_or(ClientError::NotLoopback)?;
        let mut pairs = parse_pairs(&address.query).ok_or(ClientError::Malformed)?;
        let mut path = address.path.clone();
        let mut method = "GET".to_string();
        let mut host = None;
        for change in tamper {
            match change {
                Tamper::State(v) => replace(&mut pairs, "state", v),
                Tamper::DropState => pairs.retain(|(n, _)| n != "state"),
                Tamper::DoubleParameter(name) => {
                    let value = pairs
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone());
                    match value {
                        Some(v) => pairs.push((name.clone(), v)),
                        None => {
                            pairs.push((name.clone(), String::new()));
                            pairs.push((name.clone(), String::new()));
                        }
                    }
                }
                Tamper::AddParameter(n, v) => pairs.push((n.clone(), v.clone())),
                Tamper::DropParameter(name) => pairs.retain(|(n, _)| n != name),
                Tamper::Iss(v) => replace(&mut pairs, "iss", v),
                Tamper::Path(p) => path = p.clone(),
                Tamper::Method(m) => method = m.clone(),
                Tamper::Host(h) => host = Some(h.clone()),
            }
        }
        let query = encode_pairs(pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        let target = if query.is_empty() {
            path
        } else {
            format!("{path}?{query}")
        };
        let base = format!("http://{}:{}/", address.host, address.port);
        let mut request = RawRequest::new(&method, &base)?
            .target(&target)
            .header("User-Agent", "FakeBrowser/1")
            .header("Accept", "text/html");
        if let Some(h) = host {
            request = request.host(&h);
        }
        Ok(request)
    }

    /// Follows a redirect to the listener with `tamper` applied, and returns its answer.
    pub fn follow(&self, location: &str, tamper: &[Tamper]) -> Result<HttpResponse, ClientError> {
        self.callback_request(location, tamper)?.send()
    }

    /// Plays the whole visit (14.1): authorization with `decision`, then, when Google
    /// redirects, the callback to the listener with the decision's [`Tamper`]s.
    pub fn visit(&self, address: &str, decision: Decision) -> Result<Visit, BrowserError> {
        let authorization = self.authorize(address, &decision)?;
        let (callback_address, callback) = match authorization.location.as_deref() {
            Some(location) if authorization.status == 302 => {
                match self.callback_request(location, &decision.tamper) {
                    Ok(request) => {
                        let shown = format!(
                            "http://127.0.0.1:{}{}",
                            request.port(),
                            request.request_target()
                        );
                        (Some(shown), Some(request.send()))
                    }
                    Err(e) => (None, Some(Err(e))),
                }
            }
            _ => (None, None),
        };
        Ok(Visit {
            authorization,
            callback_address,
            callback,
        })
    }

    /// A plain GET of a loopback address, as a browser sends it.
    pub fn get(&self, address: &str) -> Result<HttpResponse, ClientError> {
        RawRequest::get(address)?
            .header("User-Agent", "FakeBrowser/1")
            .header("Accept", "text/html")
            .send()
    }
}

fn replace(pairs: &mut Vec<(String, String)>, name: &str, value: &str) {
    let mut found = false;
    for (n, v) in pairs.iter_mut() {
        if n == name {
            *v = value.to_string();
            found = true;
        }
    }
    if !found {
        pairs.push((name.to_string(), value.to_string()));
    }
}
