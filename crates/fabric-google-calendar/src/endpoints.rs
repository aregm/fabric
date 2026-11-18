//! Google's fixed addresses, and the loopback set of debug builds (5.1, PD2).
//!
//! An [`Endpoints`] value can only be [`Endpoints::google`] or, in a debug build,
//! [`Endpoints::loopback`]. No function builds one from a caller's string, so no answer of a
//! server, no file and no caller can move a token to another host (3.10).

use std::fmt;

/// The scheme of an endpoint. HTTPS for Google, plain HTTP only for the loopback set (5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// `https`: the only scheme of [`Endpoints::google`].
    Https,
    /// `http`: only for the loopback set of a debug build.
    Http,
}

impl Scheme {
    /// `"https"` or `"http"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Scheme::Https => "https",
            Scheme::Http => "http",
        }
    }
}

/// Which of the five addresses an [`Endpoint`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// Where the browser goes to sign in (3.2.1). The broker sends nothing there.
    Authorization,
    /// The token endpoint (3.4, 3.8).
    Token,
    /// The revocation endpoint (3.9).
    Revocation,
    /// Google's key set for the identity token (3.5 item 3).
    Keys,
    /// The Calendar API base (section 6).
    Calendar,
}

/// One fixed address: scheme, host, port and base path. Built only inside this module.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    service: Service,
    scheme: Scheme,
    host: &'static str,
    /// `None`: the scheme's default port.
    port: Option<u16>,
    /// The base path, starting with `/` and without a trailing `/`.
    path: &'static str,
}

impl Endpoint {
    /// Which address this is.
    #[must_use]
    pub const fn service(&self) -> Service {
        self.service
    }

    /// The scheme.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The host name (`127.0.0.1` for the loopback set).
    #[must_use]
    pub const fn host(&self) -> &'static str {
        self.host
    }

    /// The port, or `None` for the scheme's default.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }

    /// The base path, starting with `/`, without a trailing `/`.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// The address as text, `scheme://host[:port]/path`. Meant for the authorization address
    /// that the browser is given (3.2.1) and for the browser opener's prefix check; never for an
    /// error or a log (5.2, PD27).
    #[must_use]
    pub fn address(&self) -> String {
        match self.port {
            Some(port) => format!(
                "{}://{}:{}{}",
                self.scheme.as_str(),
                self.host,
                port,
                self.path
            ),
            None => format!("{}://{}{}", self.scheme.as_str(), self.host, self.path),
        }
    }
}

/// Prints which service and whether it is Google's or the loopback set, never the address
/// (5.2: no address is formatted).
impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = match self.scheme {
            Scheme::Https => "google",
            Scheme::Http => "loopback",
        };
        write!(f, "Endpoint({:?}, {set})", self.service)
    }
}

/// The five addresses the adapter uses (5.1). Fields are private: the only sets are
/// [`Endpoints::google`] and, in a debug build, [`Endpoints::loopback`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Endpoints {
    authorization: Endpoint,
    token: Endpoint,
    revocation: Endpoint,
    keys: Endpoint,
    calendar: Endpoint,
}

/// The Google endpoints, as G-DOC's discovery document shows them (5.1). Constants: the
/// discovery document is not read at run time.
const fn google(service: Service, host: &'static str, path: &'static str) -> Endpoint {
    Endpoint {
        service,
        scheme: Scheme::Https,
        host,
        port: None,
        path,
    }
}

impl Endpoints {
    /// Google's five fixed HTTPS addresses (5.1).
    #[must_use]
    pub const fn google() -> Endpoints {
        Endpoints {
            authorization: google(
                Service::Authorization,
                "accounts.google.com",
                "/o/oauth2/v2/auth",
            ),
            token: google(Service::Token, "oauth2.googleapis.com", "/token"),
            revocation: google(Service::Revocation, "oauth2.googleapis.com", "/revoke"),
            keys: google(Service::Keys, "www.googleapis.com", "/oauth2/v3/certs"),
            calendar: google(Service::Calendar, "www.googleapis.com", "/calendar/v3"),
        }
    }

    /// The loopback set of a debug build: `http://127.0.0.1:<port>` with the paths the fake
    /// Google server serves (14.1). A release build does not have it (PD2). A broker may use it
    /// only with the test key source (CO10).
    #[cfg(debug_assertions)]
    #[must_use]
    pub const fn loopback(port: u16) -> Endpoints {
        const fn local(service: Service, port: u16, path: &'static str) -> Endpoint {
            Endpoint {
                service,
                scheme: Scheme::Http,
                host: "127.0.0.1",
                port: Some(port),
                path,
            }
        }
        Endpoints {
            authorization: local(Service::Authorization, port, "/o/oauth2/v2/auth"),
            token: local(Service::Token, port, "/token"),
            revocation: local(Service::Revocation, port, "/revoke"),
            keys: local(Service::Keys, port, "/oauth2/v3/certs"),
            calendar: local(Service::Calendar, port, "/calendar/v3"),
        }
    }

    /// Whether this is the loopback set (plain HTTP to `127.0.0.1`).
    #[must_use]
    pub const fn is_loopback(&self) -> bool {
        matches!(self.token.scheme, Scheme::Http)
    }

    /// The scheme every address of this set uses.
    #[must_use]
    pub const fn scheme(&self) -> Scheme {
        self.token.scheme
    }

    /// Where the browser goes to sign in (3.2.1).
    #[must_use]
    pub const fn authorization(&self) -> Endpoint {
        self.authorization
    }

    /// The token endpoint (3.4, 3.8).
    #[must_use]
    pub const fn token(&self) -> Endpoint {
        self.token
    }

    /// The revocation endpoint (3.9).
    #[must_use]
    pub const fn revocation(&self) -> Endpoint {
        self.revocation
    }

    /// The key set of the identity token (3.5 item 3).
    #[must_use]
    pub const fn keys(&self) -> Endpoint {
        self.keys
    }

    /// The Calendar API base (section 6).
    #[must_use]
    pub const fn calendar(&self) -> Endpoint {
        self.calendar
    }

    /// The endpoint of `service`.
    #[must_use]
    pub const fn get(&self, service: Service) -> Endpoint {
        match service {
            Service::Authorization => self.authorization,
            Service::Token => self.token,
            Service::Revocation => self.revocation,
            Service::Keys => self.keys,
            Service::Calendar => self.calendar,
        }
    }
}

/// Prints which set, never an address (5.2).
impl fmt::Debug for Endpoints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_loopback() {
            f.write_str("Endpoints(loopback)")
        } else {
            f.write_str("Endpoints(google)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Service; 5] = [
        Service::Authorization,
        Service::Token,
        Service::Revocation,
        Service::Keys,
        Service::Calendar,
    ];

    #[test]
    fn google_is_the_five_constants_of_section_5_1() {
        let e = Endpoints::google();
        assert_eq!(
            e.authorization().address(),
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
        assert_eq!(e.token().address(), "https://oauth2.googleapis.com/token");
        assert_eq!(
            e.revocation().address(),
            "https://oauth2.googleapis.com/revoke"
        );
        assert_eq!(
            e.keys().address(),
            "https://www.googleapis.com/oauth2/v3/certs"
        );
        assert_eq!(
            e.calendar().address(),
            "https://www.googleapis.com/calendar/v3"
        );
        assert!(!e.is_loopback());
        for service in ALL {
            let endpoint = e.get(service);
            assert_eq!(endpoint.service(), service);
            assert_eq!(endpoint.scheme(), Scheme::Https);
            assert_eq!(endpoint.port(), None);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    fn loopback_is_plain_http_to_127_0_0_1_only_with_the_fakes_paths() {
        let e = Endpoints::loopback(40_123);
        assert!(e.is_loopback());
        assert_eq!(
            e.authorization().address(),
            "http://127.0.0.1:40123/o/oauth2/v2/auth"
        );
        assert_eq!(e.token().address(), "http://127.0.0.1:40123/token");
        assert_eq!(e.revocation().address(), "http://127.0.0.1:40123/revoke");
        assert_eq!(e.keys().address(), "http://127.0.0.1:40123/oauth2/v3/certs");
        assert_eq!(e.calendar().address(), "http://127.0.0.1:40123/calendar/v3");
        for service in ALL {
            let endpoint = e.get(service);
            assert_eq!(endpoint.host(), "127.0.0.1");
            assert_eq!(endpoint.scheme(), Scheme::Http);
            assert_eq!(endpoint.port(), Some(40_123));
            assert_eq!(endpoint.path(), Endpoints::google().get(service).path());
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    fn debug_prints_no_address() {
        for e in [Endpoints::google(), Endpoints::loopback(40_123)] {
            let text = format!("{e:?} {:?} {:?}", e.token(), e.calendar());
            assert!(!text.contains("://"), "{text}");
            assert!(!text.contains("google.com") && !text.contains("googleapis"));
            assert!(!text.contains("127.0.0.1") && !text.contains("40123"));
        }
    }
}
