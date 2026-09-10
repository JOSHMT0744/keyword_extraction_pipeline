//! Lowercase-hex serialisation for the 32-byte digests in the public output.
//!
//! `[u8; 32]` serialises through serde's default path as a 32-element array of numbers,
//! which is unreadable in a log, unusable as a filename and four times the bytes. Hex is
//! the form every other tool in the chain already speaks. It round-trips, so the library
//! types stay deserialisable.

use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse exactly 32 bytes of lowercase or uppercase hex.
pub fn decode32(s: &str) -> Result<[u8; 32], &'static str> {
    if s.len() != 64 {
        return Err("expected 64 hex characters");
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(|_| "not hex")?;
        out[i] = u8::from_str_radix(pair, 16).map_err(|_| "not hex")?;
    }
    Ok(out)
}

pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&encode(bytes))
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
    let s = String::deserialize(d)?;
    decode32(&s).map_err(D::Error::custom)
}

/// `Option<[u8; 32]>`, since `hash_canonical` is absent for documents that never got far
/// enough to have canonical text.
pub mod option {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_str(&super::encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        let opt = Option::<String>::deserialize(d)?;
        match opt {
            Some(s) => super::decode32(&s).map(Some).map_err(D::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_digest() {
        let bytes: [u8; 32] = *blake3::hash(b"kep").as_bytes();
        assert_eq!(decode32(&encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn rejects_a_truncated_digest_rather_than_padding_it() {
        assert!(decode32("00ff").is_err());
        assert!(decode32(&"z".repeat(64)).is_err());
    }
}
