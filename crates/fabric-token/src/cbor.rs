//! Deterministic CBOR: RFC 8949 section 4.2.1, "Core Deterministic Encoding Requirements".
//!
//! Tokens, proof-of-possession messages and action digests sign or hash CBOR bytes, so every
//! value must have exactly one encoding. [`encode`] writes each integer and length in its
//! shortest form, uses definite lengths only, and sorts map keys by the bytewise order of their
//! encodings. [`decode`] accepts only input that is already in that form: it parses with
//! ciborium, re-encodes, and requires the same bytes back.
//!
//! The profile leaves out what the demo never sends: floating-point numbers, bignums (tags 2
//! and 3), and simple values other than `false`, `true` and `null`.

use std::fmt;

pub use ciborium::Value;
use serde::Serialize;

/// Nesting limit while decoding. Tokens nest four levels deep.
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CborError {
    /// Not well-formed CBOR, or bytes left over after the first item.
    Malformed(String),
    /// Well-formed CBOR that is not in deterministic form.
    NotDeterministic,
    /// A map with two equal keys.
    DuplicateKey,
    /// An item outside this profile, such as a floating-point number.
    Unsupported(&'static str),
    /// A Rust value that serde could not turn into CBOR.
    Serialize(String),
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(f, "malformed CBOR: {detail}"),
            Self::NotDeterministic => f.write_str("CBOR is not in deterministic form"),
            Self::DuplicateKey => f.write_str("CBOR map has a duplicate key"),
            Self::Unsupported(what) => write!(f, "CBOR profile does not allow {what}"),
            Self::Serialize(detail) => write!(f, "cannot encode as CBOR: {detail}"),
        }
    }
}

impl std::error::Error for CborError {}

/// An integer value.
#[must_use]
pub fn int(value: i64) -> Value {
    Value::Integer(value.into())
}

/// Encodes `value` deterministically.
pub fn encode(value: &Value) -> Result<Vec<u8>, CborError> {
    let mut out = Vec::new();
    write(value, &mut out)?;
    Ok(out)
}

/// The deterministic encoding of a serde value, such as an [`Action`](fabric_policy::Action).
/// Structs become maps keyed by their field names, as in their JSON form.
pub fn to_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CborError> {
    let value =
        Value::serialized(value).map_err(|error| CborError::Serialize(error.to_string()))?;
    encode(&value)
}

/// Decodes `bytes`, which must be exactly one item in deterministic form.
pub fn decode(bytes: &[u8]) -> Result<Value, CborError> {
    let mut rest = bytes;
    let value: Value = ciborium::de::from_reader_with_recursion_limit(&mut rest, MAX_DEPTH)
        .map_err(|error| CborError::Malformed(error.to_string()))?;
    if !rest.is_empty() {
        return Err(CborError::Malformed(format!(
            "{} bytes after the first item",
            rest.len()
        )));
    }
    if encode(&value)? != bytes {
        return Err(CborError::NotDeterministic);
    }
    Ok(value)
}

fn write(value: &Value, out: &mut Vec<u8>) -> Result<(), CborError> {
    match value {
        Value::Integer(integer) => {
            let value = i128::from(*integer);
            let (major, argument) = if value >= 0 {
                (0, value)
            } else {
                (1, -1 - value)
            };
            let argument = u64::try_from(argument)
                .map_err(|_| CborError::Unsupported("integers beyond 64 bits"))?;
            head(out, major, argument);
        }
        Value::Bytes(bytes) => {
            head(out, 2, length(bytes.len()));
            out.extend_from_slice(bytes);
        }
        Value::Text(text) => {
            head(out, 3, length(text.len()));
            out.extend_from_slice(text.as_bytes());
        }
        Value::Array(items) => {
            head(out, 4, length(items.len()));
            for item in items {
                write(item, out)?;
            }
        }
        Value::Map(entries) => {
            let mut encoded = entries
                .iter()
                .map(|(key, value)| Ok((encode(key)?, encode(value)?)))
                .collect::<Result<Vec<_>, CborError>>()?;
            encoded.sort_by(|left, right| left.0.cmp(&right.0));
            if encoded.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(CborError::DuplicateKey);
            }
            head(out, 5, length(encoded.len()));
            for (key, value) in encoded {
                out.extend_from_slice(&key);
                out.extend_from_slice(&value);
            }
        }
        Value::Tag(2 | 3, _) => return Err(CborError::Unsupported("bignums")),
        Value::Tag(tag, inner) => {
            head(out, 6, *tag);
            write(inner, out)?;
        }
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Null => out.push(0xf6),
        Value::Float(_) => return Err(CborError::Unsupported("floating-point numbers")),
        _ => return Err(CborError::Unsupported("this kind of item")),
    }
    Ok(())
}

/// Writes an item head with the argument in its shortest form (RFC 8949 section 4.2.1).
fn head(out: &mut Vec<u8>, major: u8, argument: u64) {
    let major = major << 5;
    if let Ok(small) = u8::try_from(argument) {
        if small < 24 {
            out.push(major | small);
        } else {
            out.extend_from_slice(&[major | 24, small]);
        }
    } else if let Ok(value) = u16::try_from(argument) {
        out.push(major | 25);
        out.extend_from_slice(&value.to_be_bytes());
    } else if let Ok(value) = u32::try_from(argument) {
        out.push(major | 26);
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.push(major | 27);
        out.extend_from_slice(&argument.to_be_bytes());
    }
}

fn length(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{CborError, Value, decode, encode, int};

    #[test]
    fn map_keys_are_sorted_by_their_encoding() {
        let map = Value::Map(vec![
            (Value::Text("b".to_owned()), int(1)),
            (int(-1), int(2)),
            (int(10), int(3)),
        ]);
        // 10 (0x0a), then -1 (0x20), then "b" (0x61 0x62).
        assert_eq!(
            encode(&map).unwrap(),
            [0xa3, 0x0a, 0x03, 0x20, 0x02, 0x61, 0x62, 0x01]
        );
    }

    #[test]
    fn duplicate_keys_and_floats_are_refused() {
        let map = Value::Map(vec![(int(1), int(1)), (int(1), int(2))]);
        assert_eq!(encode(&map), Err(CborError::DuplicateKey));
        assert!(matches!(
            encode(&Value::Float(1.5)),
            Err(CborError::Unsupported(_))
        ));
        assert_eq!(
            decode(&[0xa2, 0x01, 0x01, 0x01, 0x02]),
            Err(CborError::DuplicateKey)
        );
    }
}
