//! A JSON value that is wiped when dropped (3.2.2: what Google issued is wiped from memory).
//!
//! A token answer, the payload of an identity token and the client file (which holds the
//! client secret, 3.1) are read into a [`serde_json::Value`], whose strings are plain
//! `String`s. [`WipedJson`] owns such a value and overwrites every string in it on drop. Member
//! names are not wiped; they are Google's fixed names. Copies inside the JSON library's own
//! buffers are not reached (RK14).

use serde_json::Value;
use zeroize::Zeroize;

/// A JSON value whose strings are wiped on drop.
pub(crate) struct WipedJson(pub(crate) Value);

impl WipedJson {
    /// The value.
    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

impl Drop for WipedJson {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Overwrites every string in `value`.
fn wipe(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(members) => members.values_mut().for_each(wipe),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipe_empties_every_string_at_every_depth() {
        let mut value = serde_json::json!({"a": "x", "b": ["y", {"c": "z"}], "d": 1});
        wipe(&mut value);
        assert_eq!(
            value,
            serde_json::json!({"a": "", "b": ["", {"c": ""}], "d": 1})
        );
    }
}
