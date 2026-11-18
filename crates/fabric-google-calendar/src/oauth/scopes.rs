//! The granted scopes against the five asked for (3.6, PD8, PD9).
//!
//! The `scope` member of a token answer is split at spaces. `email` and
//! `https://www.googleapis.com/auth/userinfo.email` count as the same scope. Exactly the five
//! continue; a missing address or calendar scope is a partial grant; a missing `openid` leaves
//! no usable identity token; anything beyond the five is refused.

use std::fmt;

/// `openid`.
pub const OPENID: &str = "openid";
/// The address scope as asked for.
pub const EMAIL: &str = "email";
/// The address scope as Google may write it in a token answer.
pub const USERINFO_EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";
/// Reading the calendar list.
pub const CALENDAR_LIST: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
/// Free/busy times.
pub const FREEBUSY: &str = "https://www.googleapis.com/auth/calendar.freebusy";
/// Reading and changing events.
pub const EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";

/// The `scope` parameter of the authorization address: the five, in this order (3.2.1).
pub const ASKED: &str = "openid email https://www.googleapis.com/auth/calendar.calendarlist.readonly \
https://www.googleapis.com/auth/calendar.freebusy https://www.googleapis.com/auth/calendar.events";

/// The longest `scope` member read; a longer one is refused as more than the five.
const MAX_SCOPE_TEXT: usize = 4096;

/// One of the five scopes the adapter asks for (3.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// `openid`.
    OpenId,
    /// `email` (or `.../auth/userinfo.email`).
    Email,
    /// `.../auth/calendar.calendarlist.readonly`.
    CalendarList,
    /// `.../auth/calendar.freebusy`.
    FreeBusy,
    /// `.../auth/calendar.events`.
    Events,
}

impl Scope {
    /// The five, in the order of the authorization address.
    pub const ALL: [Scope; 5] = [
        Scope::OpenId,
        Scope::Email,
        Scope::CalendarList,
        Scope::FreeBusy,
        Scope::Events,
    ];

    /// The scope as the authorization address writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::OpenId => OPENID,
            Scope::Email => EMAIL,
            Scope::CalendarList => CALENDAR_LIST,
            Scope::FreeBusy => FREEBUSY,
            Scope::Events => EVENTS,
        }
    }

    /// The scope a granted name stands for, with `userinfo.email` read as `email`.
    #[must_use]
    pub fn from_granted(name: &str) -> Option<Scope> {
        match name {
            OPENID => Some(Scope::OpenId),
            EMAIL | USERINFO_EMAIL => Some(Scope::Email),
            CALENDAR_LIST => Some(Scope::CalendarList),
            FREEBUSY => Some(Scope::FreeBusy),
            EVENTS => Some(Scope::Events),
            _ => None,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the granted scopes allow (the table of 3.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// Exactly the five.
    Complete,
    /// The address scope or a calendar scope is missing; these are missing, in the order of
    /// [`Scope::ALL`] (PD8).
    Partial(Vec<Scope>),
    /// `openid` is missing (and nothing else): there is no usable identity token.
    NoOpenId,
    /// A scope beyond the five was granted (PD9).
    Unexpected,
}

/// Reads a `scope` member against the table of 3.6. A scope beyond the five wins over a
/// missing one: both store nothing, and the person must remove the extra access in the Google
/// account either way. A missing address or calendar scope wins over a missing `openid`.
#[must_use]
pub fn classify(scope: &str) -> Grant {
    if scope.len() > MAX_SCOPE_TEXT {
        return Grant::Unexpected;
    }
    let mut granted = [false; 5];
    for name in scope.split(' ').filter(|name| !name.is_empty()) {
        match Scope::from_granted(name) {
            Some(known) => granted[index(known)] = true,
            None => return Grant::Unexpected,
        }
    }
    let missing: Vec<Scope> = Scope::ALL
        .into_iter()
        .filter(|s| *s != Scope::OpenId && !granted[index(*s)])
        .collect();
    if !missing.is_empty() {
        Grant::Partial(missing)
    } else if !granted[index(Scope::OpenId)] {
        Grant::NoOpenId
    } else {
        Grant::Complete
    }
}

const fn index(scope: Scope) -> usize {
    match scope {
        Scope::OpenId => 0,
        Scope::Email => 1,
        Scope::CalendarList => 2,
        Scope::FreeBusy => 3,
        Scope::Events => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_asked_scopes_are_the_five_in_order() {
        let asked: Vec<&str> = ASKED.split(' ').collect();
        let all: Vec<&str> = Scope::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(asked, all);
        assert_eq!(classify(ASKED), Grant::Complete);
    }

    #[test]
    fn userinfo_email_counts_as_email_and_order_and_spaces_do_not_matter() {
        let text = format!("  {EVENTS} {USERINFO_EMAIL}  {FREEBUSY} openid {CALENDAR_LIST} ");
        assert_eq!(classify(&text), Grant::Complete);
        let doubled = format!("{ASKED} {USERINFO_EMAIL} openid");
        assert_eq!(classify(&doubled), Grant::Complete);
    }

    #[test]
    fn the_table_of_3_6() {
        assert_eq!(
            classify(&format!("openid email {CALENDAR_LIST} {FREEBUSY}")),
            Grant::Partial(vec![Scope::Events])
        );
        assert_eq!(
            classify(&format!("openid {CALENDAR_LIST} {FREEBUSY} {EVENTS}")),
            Grant::Partial(vec![Scope::Email])
        );
        assert_eq!(
            classify("openid"),
            Grant::Partial(vec![
                Scope::Email,
                Scope::CalendarList,
                Scope::FreeBusy,
                Scope::Events
            ])
        );
        assert_eq!(
            classify(&format!("email {CALENDAR_LIST} {FREEBUSY} {EVENTS}")),
            Grant::NoOpenId
        );
        for extra in [
            "profile",
            "https://www.googleapis.com/auth/calendar",
            "https://www.googleapis.com/auth/userinfo.profile",
            "OPENID",
            "https://www.googleapis.com/auth/calendar.events.owned",
        ] {
            assert_eq!(
                classify(&format!("{ASKED} {extra}")),
                Grant::Unexpected,
                "{extra}"
            );
        }
        assert_eq!(classify("profile"), Grant::Unexpected);
        assert_eq!(classify(""), Grant::Partial(Scope::ALL[1..].to_vec()));
        assert_eq!(classify(&"openid ".repeat(1000)), Grant::Unexpected);
    }

    #[test]
    fn a_tab_is_not_a_separator() {
        assert_eq!(
            classify(&format!(
                "openid\temail {CALENDAR_LIST} {FREEBUSY} {EVENTS}"
            )),
            Grant::Unexpected
        );
    }
}
