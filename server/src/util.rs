use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer};
use uuid::Uuid;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// Normalizes a client-supplied id to the canonical hyphenated form.
pub fn parse_id(s: &str) -> Option<String> {
    Uuid::parse_str(s).ok().map(|u| u.to_string())
}

/// Lets a PATCH field tell "absent" (None) apart from "null" (Some(None)).
pub fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}
