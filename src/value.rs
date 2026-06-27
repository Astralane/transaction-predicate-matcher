//! Wire-facing scalar types. All validate at deserialization so a malformed rule fails to load.

use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Base58 pubkey on the wire; parsed to `Pubkey` at load.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pk(pub Pubkey);

impl<'de> Deserialize<'de> for Pk {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Pubkey::from_str(&s).map(Pk).map_err(serde::de::Error::custom)
    }
}

/// Hex bytes on the wire (e.g. "02000000"); decoded to `Vec<u8>` at load. Any length allowed.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Bytes(pub Vec<u8>);

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map(Bytes).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxVer {
    Legacy,
    V0,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SliceKind {
    Bytes,
    U8,
    U16Le,
    U16Be,
    U32Le,
    U32Be,
    U64Le,
    U64Be,
    I64Le,
}

impl SliceKind {
    /// True for every kind except `Bytes`.
    pub fn is_numeric(&self) -> bool {
        !matches!(self, SliceKind::Bytes)
    }
}

/// Comparison target for `data_slice`. A JSON number -> `Num`, a JSON (hex) string -> `Bytes`.
/// `i128` holds any u64/i64. Deserialized by hand rather than via `#[serde(untagged)]`, because
/// untagged buffering does not support 128-bit integers (a JSON number would fail to match).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SliceVal {
    Num(i128),
    Bytes(Bytes),
}

impl<'de> Deserialize<'de> for SliceVal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = SliceVal;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an integer or a hex string")
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<SliceVal, E> {
                Ok(SliceVal::Num(v as i128))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<SliceVal, E> {
                Ok(SliceVal::Num(v as i128))
            }
            fn visit_i128<E: serde::de::Error>(self, v: i128) -> Result<SliceVal, E> {
                Ok(SliceVal::Num(v))
            }
            fn visit_u128<E: serde::de::Error>(self, v: u128) -> Result<SliceVal, E> {
                i128::try_from(v).map(SliceVal::Num).map_err(serde::de::Error::custom)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SliceVal, E> {
                hex::decode(v)
                    .map(|b| SliceVal::Bytes(Bytes(b)))
                    .map_err(serde::de::Error::custom)
            }
        }
        d.deserialize_any(V)
    }
}
