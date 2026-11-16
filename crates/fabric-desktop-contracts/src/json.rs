//! Strict JSON (README section 2) and the canonical form (README section 3).
//!
//! [`parse_strict`] is the only way bytes become a JSON value in this crate. It produces a
//! [`serde_json::Value`] whose numbers are all integers in the safe range, so every value it
//! returns has exactly one canonical form. [`canonical`] never relies on the iteration order
//! of [`serde_json::Map`]: it sorts member names itself.

use std::fmt;

use serde_json::{Map, Number, Value};

/// The largest integer every JSON implementation represents exactly, 2^53 − 1 (README section 2, rule 4).
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// The deepest nesting of arrays and objects, counting the outermost value as depth 1 (README section 2, rule 6).
pub const MAX_DEPTH: usize = 32;

/// Why bytes are not strict JSON. `offset` is the byte offset where the reader stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrictJsonError {
    pub offset: usize,
    pub reason: &'static str,
}

impl fmt::Display for StrictJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not strict JSON at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl std::error::Error for StrictJsonError {}

/// Reads bytes as strict JSON: valid UTF-8 without a byte-order mark, exactly one value
/// surrounded by optional JSON whitespace, no duplicate member names (compared after
/// unescaping), integers only within ±[`MAX_SAFE_INTEGER`] and written `-?(0|[1-9][0-9]*)`
/// with `-0` refused, no lone surrogate escapes, and at most [`MAX_DEPTH`] nested containers.
pub fn parse_strict(bytes: &[u8]) -> Result<Value, StrictJsonError> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(StrictJsonError {
            offset: 0,
            reason: "byte-order mark",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|e| StrictJsonError {
        offset: e.valid_up_to(),
        reason: "invalid UTF-8",
    })?;
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(1)?;
    parser.skip_whitespace();
    if parser.at != parser.bytes.len() {
        return Err(parser.fail("text after the value"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn fail(&self, reason: &'static str) -> StrictJsonError {
        StrictJsonError {
            offset: self.at,
            reason,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// JSON whitespace is space, tab, line feed and carriage return, nothing else (README section 2).
    fn skip_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8, reason: &'static str) -> Result<(), StrictJsonError> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.fail(reason))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, StrictJsonError> {
        match self.peek() {
            None => Err(self.fail("expected a value")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.integer(),
            Some(_) => Err(self.fail("expected a value")),
        }
    }

    fn literal(&mut self, word: &'static str, value: Value) -> Result<Value, StrictJsonError> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.fail("unknown literal"))
        }
    }

    fn integer(&mut self) -> Result<Value, StrictJsonError> {
        let start = self.at;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while let Some(b'0'..=b'9') = self.peek() {
                    self.at += 1;
                }
            }
            _ => return Err(self.fail("expected a digit")),
        }
        if let Some(b'.' | b'e' | b'E') = self.peek() {
            return Err(self.fail("a number with a fraction or an exponent"));
        }
        let digits = &self.text[start..self.at];
        if digits == "-0" {
            return Err(self.fail("negative zero"));
        }
        // 17 characters hold every safe integer with its sign; anything longer is out of range.
        if digits.len() > 17 {
            return Err(self.fail("integer outside the safe range"));
        }
        let n: i64 = digits
            .parse()
            .map_err(|_| self.fail("integer outside the safe range"))?;
        if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n) {
            return Err(self.fail("integer outside the safe range"));
        }
        Ok(Value::Number(Number::from(n)))
    }

    fn hex4(&mut self) -> Result<u32, StrictJsonError> {
        let Some(chunk) = self.bytes.get(self.at..self.at + 4) else {
            return Err(self.fail("short unicode escape"));
        };
        let mut n = 0u32;
        for &b in chunk {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(self.fail("short unicode escape")),
            };
            n = n * 16 + u32::from(digit);
        }
        self.at += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, StrictJsonError> {
        self.expect(b'"', "expected a string")?;
        let mut out = String::new();
        loop {
            let run_start = self.at;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(&self.text[run_start..self.at]);
            match self.peek() {
                None => return Err(self.fail("unterminated string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.fail("unterminated string"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => {
                            self.at -= 1;
                            return Err(self.fail("invalid escape"));
                        }
                    }
                }
                Some(_) => return Err(self.fail("raw control character in a string")),
            }
        }
    }

    /// The `XXXX` of `\uXXXX`, joined with a following low-surrogate escape when it is a
    /// high surrogate. A surrogate that is not half of a pair is refused (README section 2, rule 5).
    fn unicode_escape(&mut self) -> Result<char, StrictJsonError> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if !self.bytes[self.at..].starts_with(b"\\u") {
                    return Err(self.fail("lone surrogate"));
                }
                self.at += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(self.fail("lone surrogate"));
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(self.fail("lone surrogate")),
            _ => first,
        };
        char::from_u32(code).ok_or_else(|| self.fail("lone surrogate"))
    }

    fn enter(&self, depth: usize) -> Result<(), StrictJsonError> {
        if depth > MAX_DEPTH {
            Err(self.fail("nested deeper than 32"))
        } else {
            Ok(())
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, StrictJsonError> {
        self.enter(depth)?;
        self.expect(b'[', "expected an array")?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.fail("expected , or ]")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, StrictJsonError> {
        self.enter(depth)?;
        self.expect(b'{', "expected an object")?;
        let mut members = Map::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_whitespace();
            let name_at = self.at;
            let name = self.string()?;
            if members.contains_key(&name) {
                self.at = name_at;
                return Err(self.fail("duplicate member name"));
            }
            self.skip_whitespace();
            self.expect(b':', "expected :")?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            members.insert(name, value);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(self.fail("expected , or }")),
            }
        }
    }
}

/// The canonical form of a value (README section 3): members ordered by name in code point
/// order, no whitespace, the fixed string escapes, integers in plain decimal.
///
/// A value from [`parse_strict`] or from serializing a contract type has only integers. A
/// floating-point number, which the contract does not allow, is written as `serde_json` writes it.
#[must_use]
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            // `str` orders by UTF-8 bytes, which is the order of code points.
            let mut names: Vec<&String> = members.keys().collect();
            names.sort();
            out.push('{');
            for (i, name) in names.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(name, out);
                out.push(':');
                write_canonical(&members[name], out);
            }
            out.push('}');
        }
    }
}

/// A string in canonical form: `\"`, `\\`, the five short control escapes, `\u00xx` in
/// lowercase for the other characters below U+0020, every other character as itself.
pub fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts(text: &str) -> String {
        canonical(&parse_strict(text.as_bytes()).unwrap())
    }

    fn refuses(text: &str) -> bool {
        parse_strict(text.as_bytes()).is_err()
    }

    #[test]
    fn sorts_members_by_code_point() {
        assert_eq!(accepts(r#"{"b":1,"a":2,"B":3}"#), r#"{"B":3,"a":2,"b":1}"#);
    }

    #[test]
    fn accepts_the_safe_integer_bounds() {
        assert_eq!(
            accepts("[9007199254740991,-9007199254740991]"),
            "[9007199254740991,-9007199254740991]"
        );
    }

    #[test]
    fn refuses_numbers_outside_the_grammar() {
        for text in [
            "1.0",
            "1e2",
            "-0",
            "01",
            "+1",
            "9007199254740992",
            "-",
            "--1",
        ] {
            assert!(refuses(text), "{text}");
        }
    }

    #[test]
    fn refuses_duplicate_names_after_unescaping() {
        assert!(refuses(r#"{"a":1,"\u0061":2}"#));
    }

    #[test]
    fn refuses_lone_surrogates() {
        assert!(refuses(r#""\ud800""#));
        assert!(refuses(r#""\udc00""#));
        assert!(refuses(r#""\ud800\u0041""#));
    }

    #[test]
    fn joins_surrogate_pairs() {
        assert_eq!(accepts(r#""\ud83d\ude00""#), "\"\u{1F600}\"");
    }

    #[test]
    fn counts_depth_from_one() {
        let ok = format!("{}{}", "[".repeat(32), "]".repeat(32));
        let deep = format!("{}{}", "[".repeat(33), "]".repeat(33));
        assert!(!refuses(&ok));
        assert!(refuses(&deep));
    }

    #[test]
    fn refuses_a_byte_order_mark() {
        assert!(refuses("\u{feff}{}"));
    }

    #[test]
    fn escapes_control_characters_in_lowercase() {
        assert_eq!(accepts(r#""\u001F\u007f""#), "\"\\u001f\u{7f}\"");
    }

    #[test]
    fn refuses_trailing_text_and_empty_input() {
        assert!(refuses(""));
        assert!(refuses(" "));
        assert!(refuses("{} {}"));
        assert!(refuses("{}\u{0}"));
    }
}
