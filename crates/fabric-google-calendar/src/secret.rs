//! `Secret`: a string that is wiped on drop and never printed (section 2, 3.10, PD11).
//!
//! Tokens, the client secret, the verifier, the state, the nonce and authorization codes are
//! held as `Secret`. The type has no `Display`, its `Debug` prints `[redacted]`, and the value is
//! reached only through [`Secret::expose`], so every use is visible in the code. Copies that the
//! HTTP and JSON libraries make in their own buffers are not wiped (RK14).
//!
//! `Secret` is not `Clone`: a store that hands out a copy builds a new `Secret` from
//! [`Secret::expose`], so every copy is made on purpose.

pub(crate) mod wipe;

use std::fmt;

use zeroize::Zeroizing;

/// A string that is wiped from memory when dropped and prints as `[redacted]`.
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wraps `value`. Earlier buffers of the `String` (before a reallocation) are not wiped.
    #[must_use]
    pub fn new(value: String) -> Secret {
        Secret(Zeroizing::new(value))
    }

    /// The value. Every call site is a place where the secret leaves this type; keep the
    /// borrowed text out of every log, error and formatted value (3.10).
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    /// The length in bytes. A length is not secret for the fixed-size values this crate makes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Compares with `other` in time that depends only on the lengths, not on where the first
    /// difference is (3.3 for the state, 3.5 item 8 for the nonce). Values of different lengths
    /// are unequal at once; the length of a state or nonce is fixed and public (3.2).
    #[must_use]
    pub fn ct_eq(&self, other: &str) -> bool {
        constant_time_eq(self.expose().as_bytes(), other.as_bytes())
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Secret {
        Secret::new(value.to_owned())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Byte equality whose running time does not depend on the position of the first difference.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    std::hint::black_box(difference) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_prints_redacted_and_never_the_value() {
        let secret = Secret::from("ya29.very-secret-token");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret:#?}"), "[redacted]");
        assert_eq!(format!("{:?}", Some(&secret)), "Some([redacted])");
    }

    #[test]
    fn expose_returns_the_value() {
        let secret = Secret::new("abc".to_owned());
        assert_eq!(secret.expose(), "abc");
        assert_eq!(secret.len(), 3);
        assert!(!secret.is_empty());
        assert!(Secret::from("").is_empty());
    }

    #[test]
    fn ct_eq_is_equality() {
        let secret = Secret::from("state-0123456789");
        assert!(secret.ct_eq("state-0123456789"));
        assert!(!secret.ct_eq("state-0123456788"));
        assert!(!secret.ct_eq("xtate-0123456789"));
        assert!(!secret.ct_eq("state-012345678"));
        assert!(!secret.ct_eq("state-01234567890"));
        assert!(!secret.ct_eq(""));
        assert!(Secret::from("").ct_eq(""));
    }
}
