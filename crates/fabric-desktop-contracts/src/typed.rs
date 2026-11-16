//! Reading a JSON value into a contract type, with the schema's notion of types.
//!
//! `serde_json`'s own value deserializer lets a derived struct be read from an array of its
//! fields in order, which the schemas do not allow. [`from_value`] reads a struct only from
//! an object, a unit variant only from a string, and `()` only from `null`; everything else
//! behaves as in `serde_json`. Every typed read in this crate goes through it.

use serde::de::value::BorrowedStrDeserializer;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess,
    Visitor,
};
use serde_json::{Error, Map, Value};

/// Reads `value` as `T`. A struct must be a JSON object.
pub fn from_value<T: DeserializeOwned>(value: &Value) -> Result<T, Error> {
    T::deserialize(Strict(value))
}

struct Strict<'a>(&'a Value);

fn invalid(value: &Value, expected: &str) -> Error {
    let found = match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    };
    de::Error::custom(format!("expected {expected}, found {found}"))
}

impl<'de> Deserializer<'de> for Strict<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    visitor.visit_u64(u)
                } else if let Some(i) = n.as_i64() {
                    visitor.visit_i64(i)
                } else {
                    Err(invalid(self.0, "an integer"))
                }
            }
            Value::String(s) => visitor.visit_borrowed_str(s),
            Value::Array(items) => visitor.visit_seq(Items(items.iter())),
            Value::Object(members) => visitor.visit_map(Members::new(members)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            other => Err(invalid(other, "null")),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            Value::Object(members) => visitor.visit_map(Members::new(members)),
            other => Err(invalid(other, "an object")),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(s) => {
                visitor.visit_enum(IntoDeserializer::<Error>::into_deserializer(s.as_str()))
            }
            other => Err(invalid(other, "a string")),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit_struct seq tuple tuple_struct map identifier ignored_any
    }
}

struct Items<'a>(std::slice::Iter<'a, Value>);

impl<'de> SeqAccess<'de> for Items<'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        self.0
            .next()
            .map(|v| seed.deserialize(Strict(v)))
            .transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct Members<'a> {
    iter: serde_json::map::Iter<'a>,
    value: Option<&'a Value>,
}

impl<'a> Members<'a> {
    fn new(members: &'a Map<String, Value>) -> Self {
        Members {
            iter: members.iter(),
            value: None,
        }
    }
}

impl<'de> MapAccess<'de> for Members<'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.iter.next() {
            Some((name, value)) => {
                self.value = Some(value);
                seed.deserialize(BorrowedStrDeserializer::new(name))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| <Error as de::Error>::custom("a value without a name"))?;
        seed.deserialize(Strict(value))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Window;
    use crate::scalar::{Count, Null};
    use serde_json::json;

    #[test]
    fn a_struct_is_not_read_from_an_array() {
        let fields = json!(["2026-11-20T16:00:00.000Z", "2026-11-20T17:00:00.000Z"]);
        assert!(from_value::<Window>(&fields).is_err());
    }

    #[test]
    fn a_struct_is_read_from_an_object() {
        let window =
            json!({"start": "2026-11-20T16:00:00.000Z", "end": "2026-11-20T17:00:00.000Z"});
        assert!(from_value::<Window>(&window).is_ok());
    }

    #[test]
    fn null_is_only_null() {
        assert!(from_value::<Null>(&json!(null)).is_ok());
        assert!(from_value::<Null>(&json!([])).is_err());
    }

    #[test]
    fn an_integer_is_not_a_boolean() {
        assert!(from_value::<Count>(&json!(false)).is_err());
        assert!(from_value::<Count>(&json!(0)).is_ok());
    }
}
