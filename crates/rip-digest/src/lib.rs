//! RFC 8785 canonical bytes with bounded, interoperable JSON input.
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub const PROFILE: &str = "org.jrip.jcs-safe-v1";
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("canonical_json_invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("canonical_json_depth_exceeded")]
    Depth,
    #[error("canonical_json_number_out_of_range")]
    Integer,
}
pub fn canonical(value: &Value) -> Result<Vec<u8>, Error> {
    fn check(v: &Value, depth: usize) -> Result<(), Error> {
        if depth > 64 {
            return Err(Error::Depth);
        }
        match v {
            Value::Number(n) => {
                if n.as_f64()
                    .is_none_or(|v| !v.is_finite() || v.abs() > 9_007_199_254_740_991.0)
                {
                    return Err(Error::Integer);
                }
            }
            Value::Array(a) => {
                for v in a {
                    check(v, depth + 1)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    check(v, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    check(value, 0)?;
    Ok(serde_jcs::to_vec(value)?)
}
pub fn digest(value: &Value) -> Result<String, Error> {
    Ok(format!("sha256:{:x}", Sha256::digest(canonical(value)?)))
}
/// Parse original JSON without silently discarding duplicate object properties.
pub fn parse(bytes: &[u8]) -> Result<Value, Error> {
    let value = serde_json::from_slice::<Strict>(bytes)?.0;
    canonical(&value)?;
    Ok(value)
}
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate properties")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element::<Strict>()? {
                    out.push(v.0);
                }
                Ok(Strict(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom("duplicate property"));
                    }
                    out.insert(k, a.next_value::<Strict>()?.0);
                }
                Ok(Strict(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_numbers_utf16_sort_and_strict_inputs() {
        let value: Value = serde_json::from_slice(
            br#"{"numbers":[333333333.33333329,1E30,4.50,2e-3,1e-27,-0.0]}"#,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(serde_jcs::to_vec(&value).unwrap()).unwrap(),
            "{\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27,0]}"
        );
        let value = serde_json::json!({"\u{e000}":1,"😀":2,"a":3});
        assert_eq!(
            String::from_utf8(canonical(&value).unwrap()).unwrap(),
            "{\"a\":3,\"😀\":2,\"\u{e000}\":1}"
        );
        assert!(parse(br#"{"a":1,"\u0061":2}"#).is_err());
        assert!(parse(br#"{"nested":{"x":1,"x":2}}"#).is_err());
        assert!(parse(b"9007199254740993").is_err());
        assert!(parse(b"NaN").is_err());
        assert!(parse(br#""\ud800""#).is_err());
        assert_eq!(
            digest(&parse(br#"{"b":1.0,"a":0}"#).unwrap()).unwrap(),
            digest(&parse(br#"{"a":-0.0,"b":1}"#).unwrap()).unwrap()
        );
    }
}
