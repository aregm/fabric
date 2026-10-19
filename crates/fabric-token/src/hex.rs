//! Lowercase hexadecimal for keys, digests and nonces in JSON files and audit records.

const DIGITS: &[u8; 16] = b"0123456789abcdef";

#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Decodes lowercase hex.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!(
            "{} hex digits; expected an even number",
            text.len()
        ));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((digit(pair[0])? << 4) | digit(pair[1])?))
        .collect()
}

/// Decodes lowercase hex of exactly `N` bytes.
pub fn decode_array<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let bytes = decode(text)?;
    <[u8; N]>::try_from(bytes.as_slice())
        .map_err(|_| format!("{} bytes; expected {N}", bytes.len()))
}

fn digit(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(format!(
            "{:?} is not a lowercase hex digit",
            char::from(byte)
        )),
    }
}

/// Serde support for byte arrays as lowercase hex strings: `#[serde(with = "hex::array")]`.
pub mod array {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let text = String::deserialize(deserializer)?;
        super::decode_array(&text).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, decode_array, encode};

    #[test]
    fn round_trips_and_rejects_bad_input() {
        assert_eq!(encode(&[0x00, 0x7f, 0xff]), "007fff");
        assert_eq!(decode("007fff").unwrap(), [0x00, 0x7f, 0xff]);
        assert!(decode("0").is_err());
        assert!(decode("0g").is_err());
        assert!(decode("FF").is_err());
        assert!(decode_array::<2>("00ff00").is_err());
        assert_eq!(decode_array::<2>("00ff").unwrap(), [0x00, 0xff]);
    }
}
