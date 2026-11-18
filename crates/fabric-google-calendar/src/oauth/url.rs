//! The authorization address the browser is given (3.2.1).
//!
//! The authorization endpoint of the [`Endpoints`] set with exactly the parameters of 3.2.1 and
//! no others: `client_id`, `redirect_uri`, `response_type=code`, `scope` (the five),
//! `code_challenge`, `code_challenge_method=S256`, `state`, `nonce`, then `prompt=select_account`
//! only for a new account, or `login_hint=<subject>` only for a reconnect. It is built with
//! [`browser_address`], which encodes every name and value (5.3), and is a [`Secret`] because it
//! holds the state and the nonce.

use crate::endpoints::Endpoints;
use crate::http::browser_address;
use crate::secret::Secret;

use super::scopes;

/// What the sign-in is for, as far as the address is concerned (3.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressPurpose<'a> {
    /// A new account: `prompt=select_account`, so the person chooses the account.
    NewAccount,
    /// A reconnect of a known account: `login_hint=<the stored subject>` (G-DOC: an address or
    /// a `sub`; the subject keeps the address out of the URL).
    Reconnect {
        /// The account's stored subject.
        subject: &'a str,
    },
}

/// `http://127.0.0.1:<port>`, no path: the redirect of the loopback listener (3.2.1, PD6). The
/// token request sends the same string (3.4).
#[must_use]
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The authorization address of 3.2.1.
#[must_use]
pub fn authorization_address(
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &Secret,
    nonce: &Secret,
    purpose: AddressPurpose<'_>,
) -> Secret {
    let mut pairs: Vec<(&'static str, &str)> = vec![
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", scopes::ASKED),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state.expose()),
        ("nonce", nonce.expose()),
    ];
    match purpose {
        AddressPurpose::NewAccount => pairs.push(("prompt", "select_account")),
        AddressPurpose::Reconnect { subject } => pairs.push(("login_hint", subject)),
    }
    browser_address(&endpoints.authorization(), &pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(address: &Secret) -> Vec<String> {
        let (_, query) = address.expose().split_once('?').expect("a query");
        query
            .split('&')
            .map(|pair| pair.split_once('=').expect("a pair").0.to_owned())
            .collect()
    }

    #[test]
    fn a_new_account_asks_to_select_the_account_and_gives_no_hint() {
        let address = authorization_address(
            &Endpoints::google(),
            "client-1.apps.googleusercontent.com",
            &redirect_uri(50_123),
            "challenge",
            &Secret::from("state-x"),
            &Secret::from("nonce-y"),
            AddressPurpose::NewAccount,
        );
        assert!(
            address
                .expose()
                .starts_with("https://accounts.google.com/o/oauth2/v2/auth?")
        );
        assert_eq!(
            names(&address),
            [
                "client_id",
                "redirect_uri",
                "response_type",
                "scope",
                "code_challenge",
                "code_challenge_method",
                "state",
                "nonce",
                "prompt"
            ]
        );
        assert!(address.expose().contains("&prompt=select_account"));
        assert!(
            address
                .expose()
                .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A50123&")
        );
        assert!(address.expose().contains("scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.calendarlist.readonly%20"));
    }

    #[test]
    fn a_reconnect_hints_the_subject_and_does_not_prompt() {
        let address = authorization_address(
            &Endpoints::google(),
            "client-1",
            &redirect_uri(1),
            "challenge",
            &Secret::from("s"),
            &Secret::from("n"),
            AddressPurpose::Reconnect {
                subject: "1234567890",
            },
        );
        let names = names(&address);
        assert_eq!(names.last().map(String::as_str), Some("login_hint"));
        assert!(!names.iter().any(|n| n == "prompt"));
        assert!(address.expose().ends_with("&login_hint=1234567890"));
    }
}
