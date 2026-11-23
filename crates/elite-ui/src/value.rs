//! The value of a movie message's argument (ARCHITECTURE.md A10): what Elite's exe hands a movie and
//! gets back, as ActionScript sees it: null, a boolean, a number, a string, an array or an object.
//! An object keeps its fields in the order they were given. Serde reads and writes it as the plain
//! JSON shape (`null`, `true`, `1.5`, `"text"`, `[...]`, `{...}`), fields in order, so messages can
//! be written as JSON in tests.

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<Value>),
    /// Fields in the order given.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// An array of anything that becomes a value.
    pub fn array<T: Into<Value>>(items: impl IntoIterator<Item = T>) -> Self {
        Value::Array(items.into_iter().map(Into::into).collect())
    }

    /// An object from (name, value) pairs, in the order given.
    pub fn object<K: Into<String>, T: Into<Value>>(fields: impl IntoIterator<Item = (K, T)>) -> Self {
        Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }

    /// The first field of that name, if this is an object.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == name).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}

impl From<f32> for Value {
    fn from(n: f32) -> Self {
        Value::Num(f64::from(n))
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Num(f64::from(n))
    }
}

impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Num(f64::from(n))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Value::Array(items)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Num(n) => s.serialize_f64(*n),
            Value::Str(t) => s.serialize_str(t),
            Value::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Object(fields) => {
                let mut map = s.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("null, a boolean, a number, a string, an array or an object")
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Value::deserialize(d)
    }

    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }

    fn visit_i64<E>(self, n: i64) -> Result<Value, E> {
        Ok(Value::Num(n as f64))
    }

    fn visit_u64<E>(self, n: u64) -> Result<Value, E> {
        Ok(Value::Num(n as f64))
    }

    fn visit_f64<E>(self, n: f64) -> Result<Value, E> {
        Ok(Value::Num(n))
    }

    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::Str(s.to_string()))
    }

    fn visit_string<E>(self, s: String) -> Result<Value, E> {
        Ok(Value::Str(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            fields.push((k, v));
        }
        Ok(Value::Object(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn constructors_and_accessors() {
        let v = Value::object([("speedProp", Value::from(0.5)), ("speedString", "$UNIT:#value=80;".into())]);
        assert_eq!(v.get("speedProp").and_then(Value::as_f64), Some(0.5));
        assert_eq!(v.get("speedString").and_then(Value::as_str), Some("$UNIT:#value=80;"));
        assert_eq!(v.get("missing"), None);
        assert_eq!(Value::array([1, 2]), Value::Array(vec![Value::Num(1.0), Value::Num(2.0)]));
        assert_eq!(Value::from(None::<bool>), Value::Null);
        assert_eq!(Value::from(Some(true)).as_bool(), Some(true));
        assert_eq!(Value::from(3u32).as_f64(), Some(3.0));
    }

    #[test]
    fn json_keeps_the_fields_in_order() {
        let text = r#"{"z":1,"a":[true,null,"x"],"m":{"b":2.5,"a":-1}}"#;
        let v: Value = serde_json::from_str(text).unwrap();
        let Value::Object(fields) = &v else { panic!("an object") };
        let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["z", "a", "m"]);
        assert_eq!(v.get("a"), Some(&Value::array([Value::Bool(true), Value::Null, "x".into()])));
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"z":1.0,"a":[true,null,"x"],"m":{"b":2.5,"a":-1.0}}"#);
    }
}
