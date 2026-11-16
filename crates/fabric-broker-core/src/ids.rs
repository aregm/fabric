//! New ids from the system random source.

use fabric_desktop_contracts::scalar::Id;

/// The digits of a provider event id: base32hex in lowercase, the alphabet Google accepts.
const BASE32HEX: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

/// 128 bits from the system random source.
///
/// # Panics
///
/// When the system random source fails, as `fabric_storage::ledger::operation_id` does.
fn random_bits() -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the system random source gives 16 bytes");
    bytes
}

/// prefix + "-" + 32 lowercase hex.
///
/// # Panics
///
/// When the system random source fails, or when `prefix` makes an invalid id (it must start
/// with a letter or digit and hold only letters, digits, `.`, `_` and `-`).
#[must_use]
pub fn new_id(prefix: &str) -> Id {
    let hex = fabric_desktop_contracts::digest::hex(&random_bits());
    Id::new(format!("{prefix}-{hex}")).expect("a short prefix and 32 hex digits make an id")
}

/// 26 chars of [0-9a-v] from 128 bits.
///
/// # Panics
///
/// When the system random source fails.
#[must_use]
pub fn provider_event_id() -> String {
    encode_base32hex(u128::from_be_bytes(random_bits()))
}

/// The 26 base32hex digits of `value`, most significant first.
fn encode_base32hex(value: u128) -> String {
    (0..26)
        .rev()
        .map(|digit| char::from(BASE32HEX[((value >> (digit * 5)) & 0x1f) as usize]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_id_is_the_prefix_and_32_hex_digits() {
        let id = new_id("request");
        let text = id.as_str();
        assert_eq!(text.len(), "request-".len() + 32);
        assert!(text.starts_with("request-"));
        assert!(
            text["request-".len()..]
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_ne!(new_id("request"), id);
    }

    #[test]
    fn a_provider_event_id_is_26_base32hex_digits() {
        let id = provider_event_id();
        assert_eq!(id.len(), 26);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c))
        );
        assert_ne!(provider_event_id(), id);
    }

    #[test]
    fn base32hex_writes_every_bit() {
        assert_eq!(encode_base32hex(0), "0".repeat(26));
        assert_eq!(encode_base32hex(31), format!("{}v", "0".repeat(25)));
        let all = encode_base32hex(u128::MAX);
        assert_eq!(&all[..1], "7");
        assert_eq!(&all[1..], "v".repeat(25));
    }
}
