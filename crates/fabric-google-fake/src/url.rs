//! Percent-encoding, query strings and form bodies.

/// Decodes `%XX` sequences, and `+` as a space when `plus_is_space` (a query or a form body).
/// `None` when a sequence is broken or the result is not UTF-8.
pub fn percent_decode(s: &str, plus_is_space: bool) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let text = std::str::from_utf8(hex).ok()?;
                if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                out.push(u8::from_str_radix(text, 16).ok()?);
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Encodes every byte except `A-Z a-z 0-9 - . _ ~` as `%XX`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Splits a query string or a form body into decoded pairs, in order, duplicates kept. A pair
/// without `=` has an empty value. `None` when any part does not decode.
pub fn parse_pairs(s: &str) -> Option<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for part in s.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        pairs.push((percent_decode(name, true)?, percent_decode(value, true)?));
    }
    Some(pairs)
}

/// Joins pairs into a query string, each name and value percent-encoded.
pub fn encode_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    pairs
        .into_iter()
        .map(|(n, v)| format!("{}={}", percent_encode(n), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// The pieces of an `http://host:port/path?query` address. Only plain `http` is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpAddress {
    /// The host as written (`127.0.0.1`, `localhost`, `[::1]`, ...).
    pub host: String,
    /// The port; 80 when the address names none.
    pub port: u16,
    /// The path, `/` when the address has none.
    pub path: String,
    /// The query without its `?`, empty when there is none.
    pub query: String,
}

impl HttpAddress {
    /// Reads an `http://` address. No user part, no fragment.
    pub fn parse(address: &str) -> Option<HttpAddress> {
        let rest = address.strip_prefix("http://")?;
        if rest.contains('#') {
            return None;
        }
        let split = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(split);
        if authority.is_empty() || authority.contains('@') {
            return None;
        }
        let (host, port) = if let Some(after) = authority.strip_prefix('[') {
            let close = after.find(']')?;
            let host = format!("[{}]", &after[..close]);
            let rest = &after[close + 1..];
            let port = match rest.strip_prefix(':') {
                Some(p) => p.parse().ok()?,
                None if rest.is_empty() => 80,
                None => return None,
            };
            (host, port)
        } else {
            match authority.split_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().ok()?),
                None => (authority.to_string(), 80),
            }
        };
        let (path, query) = match tail.split_once('?') {
            Some((p, q)) => (p, q),
            None => (tail, ""),
        };
        let path = if path.is_empty() { "/" } else { path };
        Some(HttpAddress {
            host,
            port,
            path: path.to_string(),
            query: query.to_string(),
        })
    }

    /// The request target: the path, and `?query` when there is a query.
    pub fn target(&self) -> String {
        if self.query.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, self.query)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_decode_plus_and_percent_and_keep_duplicates() {
        let pairs = parse_pairs("a=1+2&b=%40x&a=3&c").unwrap();
        assert_eq!(
            pairs,
            vec![
                ("a".into(), "1 2".into()),
                ("b".into(), "@x".into()),
                ("a".into(), "3".into()),
                ("c".into(), "".into()),
            ]
        );
        assert!(parse_pairs("a=%zz").is_none());
    }

    #[test]
    fn addresses_are_split_into_host_port_path_and_query() {
        let a = HttpAddress::parse("http://127.0.0.1:8080?x=1").unwrap();
        assert_eq!(
            (a.host.as_str(), a.port, a.path.as_str()),
            ("127.0.0.1", 8080, "/")
        );
        assert_eq!(a.query, "x=1");
        let b = HttpAddress::parse("http://[::1]:9/p").unwrap();
        assert_eq!(
            (b.host.as_str(), b.port, b.path.as_str()),
            ("[::1]", 9, "/p")
        );
        assert!(HttpAddress::parse("https://127.0.0.1/").is_none());
        assert!(HttpAddress::parse("http://u@127.0.0.1/").is_none());
    }
}
