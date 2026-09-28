//! Client record ids (D7).
//!
//! Every row a client can create carries a `client_record_id`, unique across the
//! whole system, so that a record created twice -- a double-tapped Save, a post
//! retried after a timeout, or in phase 3 an offline outbox replaying on
//! reconnect -- is stored once.
//!
//! The ids are UUIDv7 (RFC 9562): a millisecond timestamp followed by random
//! bits, so ids from different devices never collide and still sort in roughly
//! the order they were made.
//!
//! The clock and the randomness are parameters, not calls, so this module stays
//! wasm-clean and a browser client can mint ids with exactly the same code.

/// Format a UUIDv7 from a Unix time in milliseconds and ten random bytes.
pub fn uuid_v7(unix_ms: u64, random: [u8; 10]) -> String {
    let mut bytes = [0u8; 16];
    // 48-bit big-endian timestamp. Overflows in the year 10889.
    bytes[..6].copy_from_slice(&unix_ms.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&random);
    bytes[6] = (bytes[6] & 0x0f) | 0x70; // version 7
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 9562 variant

    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A client-supplied record id in canonical lowercase form, or `None` if it is
/// not a UUID.
///
/// Any UUID version is accepted: what matters server-side is that the value is
/// bounded, unique, and safe to store, not which algorithm the client used.
pub fn normalize(raw: &str) -> Option<String> {
    let id = raw.trim().to_ascii_lowercase();
    let well_formed = id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    well_formed.then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v7_id_is_canonical_and_carries_its_version_and_variant() {
        let id = uuid_v7(0x0191_f7ac_1234, [0xff; 10]);
        assert_eq!(id, "0191f7ac-1234-7fff-bfff-ffffffffffff");
        assert_eq!(normalize(&id).as_deref(), Some(id.as_str()));
    }

    #[test]
    fn later_ids_sort_after_earlier_ones() {
        // The point of v7 over v4: rows keyed by these ids come back in the
        // order they were created, whichever device made them.
        let earlier = uuid_v7(1_760_000_000_000, [0xff; 10]);
        let later = uuid_v7(1_760_000_000_001, [0x00; 10]);
        assert!(earlier < later);
    }

    #[test]
    fn the_random_bits_distinguish_ids_from_the_same_millisecond() {
        assert_ne!(uuid_v7(42, [1; 10]), uuid_v7(42, [2; 10]));
    }

    #[test]
    fn normalizing_accepts_any_uuid_and_lowercases_it() {
        assert_eq!(
            normalize("  0191F7AC-1234-4000-8000-ABCDEFABCDEF ").as_deref(),
            Some("0191f7ac-1234-4000-8000-abcdefabcdef")
        );
    }

    #[test]
    fn normalizing_rejects_anything_that_is_not_a_uuid() {
        for bad in [
            "",
            "not-a-uuid",
            "0191f7ac12344000800abcdefabcdef000",
            "0191f7ac-1234-4000-8000-abcdefabcdeg",
            "0191f7ac-1234-4000-8000-abcdefabcdef0",
            "0191f7ac_1234_4000_8000_abcdefabcdef",
            "'; DROP TABLE observations; --",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
    }
}
