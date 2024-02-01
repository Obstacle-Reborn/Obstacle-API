use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use async_graphql::{Scalar, ScalarType};
use deadpool_redis::redis::ToRedisArgs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::{Database, Decode};

static ESCAPE_CHARS: Lazy<HashMap<u8, &'static str>> = Lazy::new(|| {
    HashMap::from([
        (b'<', "&lt;"),
        (b'>', "&gt;"),
        (b'&', "&amp;"),
        (b'\'', "&#39;"),
        (b'"', "&quot;"),
    ])
});

fn escape(s: String) -> String {
    let mut out = String::new();
    let pile_o_bits = &s;
    let mut last = 0;
    for (i, ch) in s.bytes().enumerate() {
        if ESCAPE_CHARS.contains_key(&ch) {
            out.push_str(&pile_o_bits[last..i]);
            out.push_str(ESCAPE_CHARS[&ch]);
            last = i + 1;
        }
    }

    if last < s.len() {
        out.push_str(&pile_o_bits[last..]);
    }

    out
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(from = "String")]
pub struct Escaped(pub String);

impl ToRedisArgs for Escaped {
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + deadpool_redis::redis::RedisWrite,
    {
        self.0.write_redis_args(out)
    }
}

impl<'r, DB: Database> Decode<'r, DB> for Escaped
where
    String: Decode<'r, DB>,
{
    fn decode(
        value: <DB as sqlx::database::HasValueRef<'r>>::ValueRef,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let value = <String as Decode<DB>>::decode(value)?;
        Ok(value.into())
    }
}

#[Scalar]
impl ScalarType for Escaped {
    fn parse(value: async_graphql::Value) -> async_graphql::InputValueResult<Self> {
        <String as ScalarType>::parse(value)
            .map(|s| s.into())
            .map_err(|e| e.propagate())
    }

    fn to_value(&self) -> async_graphql::Value {
        <String as ScalarType>::to_value(&self.0)
    }
}

impl From<String> for Escaped {
    fn from(value: String) -> Self {
        Self(escape(value))
    }
}

impl From<Escaped> for String {
    fn from(value: Escaped) -> Self {
        escape(value.0)
    }
}

impl Deref for Escaped {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Escaped {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
