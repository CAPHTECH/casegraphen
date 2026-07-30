use crate::native_model::{CaseSpace, MorphismLogEntry};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256 as Sha2Sha256};
use std::fmt::Write as _;

pub(crate) fn case_space_checksum(case_space: &CaseSpace) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(case_space)?;
    if let Value::Object(object) = &mut value {
        if let Some(Value::Object(revision)) = object.get_mut("revision") {
            revision.insert("checksum".to_owned(), Value::String(String::new()));
        }
        if let Some(Value::Array(log)) = object.get_mut("morphism_log") {
            for entry in log {
                if let Value::Object(entry) = entry {
                    entry.insert("replay_checksum".to_owned(), Value::String(String::new()));
                }
            }
        }
    }
    let canonical = canonical_json(&value)?;
    Ok(format!("sha256:{}", sha256_hex(canonical.as_bytes())))
}

pub(crate) fn morphism_log_entry_hash(
    entry: &MorphismLogEntry,
) -> Result<String, serde_json::Error> {
    canonical_json_sha256(entry)
}

fn canonical_json(value: &impl Serialize) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

fn canonical_json_sha256(value: &impl Serialize) -> Result<String, serde_json::Error> {
    let canonical = canonical_json(value)?;
    Ok(sha256_hex(canonical.as_bytes()))
}

#[derive(Clone)]
pub(crate) struct Sha256(Sha2Sha256);

impl Sha256 {
    pub(crate) fn new() -> Self {
        Self(Sha2Sha256::new())
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub(crate) fn finalize_hex(self) -> String {
        self.0
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut digest, byte| {
                let _ = write!(digest, "{byte:02x}");
                digest
            })
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize_hex()
}

#[cfg(test)]
mod tests {
    use super::{sha256_hex, Sha256};

    #[test]
    fn sha256_matches_the_standard_abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_matches_empty_and_block_boundary_vectors() {
        for (message, expected) in [
            (
                Vec::new(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                vec![b'a'; 64],
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
            (
                vec![b'a'; 65],
                "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
            ),
        ] {
            assert_eq!(sha256_hex(&message), expected);
        }
    }

    #[test]
    fn sha256_matches_nist_and_long_multi_block_vectors() {
        let nist_896_bit_message = concat!(
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmn",
            "hijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        );
        assert_eq!(nist_896_bit_message.len(), 112);
        assert_eq!(
            sha256_hex(nist_896_bit_message.as_bytes()),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 200]),
            "c2a908d98f5df987ade41b5fce213067efbcc21ef2240212a41e54b5e7c28ae5"
        );
    }

    #[test]
    fn sha256_streaming_updates_match_one_shot_hashing() {
        let mut hasher = Sha256::new();
        hasher.update(b"a");
        hasher.update(b"b");
        hasher.update(b"c");

        assert_eq!(hasher.finalize_hex(), sha256_hex(b"abc"));
    }

    #[test]
    fn sha256_streaming_matches_one_shot_across_block_boundaries() {
        for byte_len in [0, 1, 55, 56, 63, 64, 65, 127, 128, 129, 8193] {
            let bytes = (0..byte_len)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            let mut hasher = Sha256::new();
            for chunk in bytes.chunks(7) {
                hasher.update(chunk);
            }

            assert_eq!(
                hasher.finalize_hex(),
                sha256_hex(&bytes),
                "streaming mismatch for {byte_len} bytes"
            );
        }
    }
}
