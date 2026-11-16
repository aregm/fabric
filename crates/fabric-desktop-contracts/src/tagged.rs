//! Helpers for the schemas' `oneOf` objects whose branches differ by one `const` member.
//!
//! A branch holds exactly when its tag member has the branch's value and the other members
//! match the branch body. The helpers split the tag off, pick the body by the tag, and read
//! the body with `deny_unknown_fields`, so exactly one branch can hold.

use serde::de::{self, DeserializeOwned, Deserializer};
use serde::ser::{self, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Reads an object and removes its string member `tag`. Returns the tag and the other members.
pub(crate) fn split<'de, D: Deserializer<'de>>(
    deserializer: D,
    tag: &'static str,
) -> Result<(String, Map<String, Value>), D::Error> {
    let value = Value::deserialize(deserializer)?;
    let Value::Object(mut members) = value else {
        return Err(de::Error::custom("expected an object"));
    };
    match members.remove(tag) {
        Some(Value::String(name)) => Ok((name, members)),
        Some(_) => Err(de::Error::custom(format!("{tag} is not a string"))),
        None => Err(de::Error::missing_field(tag)),
    }
}

/// Reads the members left after the tag as the branch body `T`.
pub(crate) fn body<T: DeserializeOwned, E: de::Error>(rest: Map<String, Value>) -> Result<T, E> {
    crate::typed::from_value(&Value::Object(rest)).map_err(E::custom)
}

/// Requires that nothing is left after the tag, for a branch that is the tag alone.
pub(crate) fn nothing<E: de::Error>(rest: &Map<String, Value>) -> Result<(), E> {
    match rest.keys().next() {
        None => Ok(()),
        Some(name) => Err(E::custom(format!("unknown member {name}"))),
    }
}

/// Removes the required member `name` and returns it.
pub(crate) fn take<E: de::Error>(
    rest: &mut Map<String, Value>,
    name: &'static str,
) -> Result<Value, E> {
    rest.remove(name).ok_or_else(|| E::missing_field(name))
}

/// Reads the value of one member as `T`.
pub(crate) fn read<T: DeserializeOwned, E: de::Error>(value: Value) -> Result<T, E> {
    crate::typed::from_value(&value).map_err(E::custom)
}

pub(crate) fn unknown<E: de::Error>(tag: &str, value: &str) -> E {
    E::custom(format!("no branch has {tag} {value:?}"))
}

/// Writes the body's members with the tag member added.
pub(crate) fn join<S: Serializer, T: Serialize + ?Sized>(
    serializer: S,
    tag: &'static str,
    name: &str,
    body: &T,
) -> Result<S::Ok, S::Error> {
    let mut members = match serde_json::to_value(body).map_err(ser::Error::custom)? {
        Value::Object(members) => members,
        Value::Null => Map::new(),
        _ => return Err(ser::Error::custom("a tagged body must be an object")),
    };
    members.insert(tag.to_owned(), Value::String(name.to_owned()));
    Value::Object(members).serialize(serializer)
}

/// The body of a branch that is its tag alone.
#[derive(Serialize)]
pub(crate) struct Empty {}
