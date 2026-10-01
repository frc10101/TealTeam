//! Offline auth tokens (C9): who is signed in on a device, signed by the Pi.
//!
//! A session is a row on the Pi, so a device with no signal cannot ask who
//! its scout is, and a session that lapsed overnight leaves the outbox (C7)
//! with no one to push as. A token is the answer carried on the device: a
//! [PASETO] `v4.public` token, Ed25519 over the [`Claims`], issued while the
//! session is live and good for [`TOKEN_DURATION`].
//!
//! - **The device reads it.** The claims are plain JSON in the token, so a
//!   page or worker decides what to show with no request and no key.
//! - **The Pi verifies it, on every write.** A sync that comes with a token
//!   and no live session is taken only if the signature is the Pi's, it has
//!   not expired, and it names the tablet whose device cookie came with it.
//!   The token is layered on device identity (A5): copied to another
//!   browser, it is refused there.
//! - **It cannot be revoked.** The plan accepts this for a three-day event
//!   with a known roster. The Pi does load the user again, so a deleted
//!   account's token is refused, and roles come from the database, not the
//!   claims.
//!
//! Pure Rust (`ed25519-dalek`), so the browser's wasm verifies a token
//! exactly as the Pi does, without `crypto.subtle`, which a browser offers
//! only over https (open decision 9).
//!
//! [PASETO]: https://github.com/paseto-standard/paseto-spec

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::user::{Roles, User};

/// How long a token is good for: the plan's 48-72 hours, at the long end, so
/// a token taken on the Friday morning of an event lasts to Sunday's finals.
pub const TOKEN_DURATION: TimeDelta = TimeDelta::hours(72);

/// What every token this module makes or takes begins with.
pub const HEADER: &str = "v4.public.";

const SIGNATURE_LEN: usize = 64;

/// What a token says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The user's id, as PASETO's `sub`, which is a string.
    #[serde(rename = "sub", with = "subject")]
    pub user_id: i64,
    pub name: String,
    pub team: Option<i32>,
    pub roles: Roles,
    /// The device UUID the token was issued to (A5).
    #[serde(rename = "dev")]
    pub device: String,
    #[serde(rename = "iat")]
    pub issued_at: DateTime<Utc>,
    #[serde(rename = "exp")]
    pub expires_at: DateTime<Utc>,
}

impl Claims {
    /// A token's claims for `user` on `device`, from `now`, to the second.
    pub fn new(user: &User, device: &str, now: DateTime<Utc>) -> Self {
        let now = DateTime::from_timestamp(now.timestamp(), 0).unwrap_or(now);
        Self {
            user_id: user.id,
            name: user.name.clone(),
            team: user.team_number,
            roles: user.roles,
            device: device.to_string(),
            issued_at: now,
            expires_at: now + TOKEN_DURATION,
        }
    }
}

/// Why a token was not taken.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("not a v4.public token")]
    Malformed,
    #[error("the signature is not the Pi's")]
    BadSignature,
    #[error("the token has expired")]
    Expired,
}

/// The public half of a 32-byte Ed25519 seed: what a device needs to verify.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// A token carrying `claims`, signed with `seed`.
pub fn sign(seed: &[u8; 32], claims: &Claims) -> String {
    let message = serde_json::to_vec(claims).expect("claims serialize");
    sign_raw(seed, &message, b"", b"")
}

/// The claims of `token`, if `public_key` signed it and it is live at `now`.
pub fn verify(
    public_key: &[u8; 32],
    token: &str,
    now: DateTime<Utc>,
) -> Result<Claims, TokenError> {
    let (message, _footer) = verify_raw(public_key, token, b"")?;
    let claims: Claims = serde_json::from_slice(&message).map_err(|_| TokenError::Malformed)?;
    if now >= claims.expires_at {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

/// The claims of `token` without checking its signature or expiry: what a
/// device does to decide what to show. Never for deciding what to accept.
pub fn read_unverified(token: &str) -> Result<Claims, TokenError> {
    let (signed, _) = split(token)?;
    serde_json::from_slice(&signed[..signed.len() - SIGNATURE_LEN])
        .map_err(|_| TokenError::Malformed)
}

/// PASETO v4.public `Sign`, with a footer and an implicit assertion.
fn sign_raw(seed: &[u8; 32], message: &[u8], footer: &[u8], implicit: &[u8]) -> String {
    let key = SigningKey::from_bytes(seed);
    let signature = key.sign(&pae(&[HEADER.as_bytes(), message, footer, implicit]));
    let mut body = message.to_vec();
    body.extend_from_slice(&signature.to_bytes());
    let mut token = format!("{HEADER}{}", URL_SAFE_NO_PAD.encode(body));
    if !footer.is_empty() {
        token.push('.');
        token.push_str(&URL_SAFE_NO_PAD.encode(footer));
    }
    token
}

/// PASETO v4.public `Verify`: the message and footer, if the signature holds.
fn verify_raw(
    public_key: &[u8; 32],
    token: &str,
    implicit: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), TokenError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| TokenError::BadSignature)?;
    let (signed, footer) = split(token)?;
    let (message, signature) = signed.split_at(signed.len() - SIGNATURE_LEN);
    let signature = Signature::from_slice(signature).map_err(|_| TokenError::Malformed)?;
    key.verify_strict(
        &pae(&[HEADER.as_bytes(), message, &footer, implicit]),
        &signature,
    )
    .map_err(|_| TokenError::BadSignature)?;
    Ok((message.to_vec(), footer))
}

/// The signed body (message, then signature) and the footer.
fn split(token: &str) -> Result<(Vec<u8>, Vec<u8>), TokenError> {
    let rest = token.strip_prefix(HEADER).ok_or(TokenError::Malformed)?;
    let (body, footer) = match rest.split_once('.') {
        Some((body, footer)) => (body, footer),
        None => (rest, ""),
    };
    let decode = |s: &str| URL_SAFE_NO_PAD.decode(s).map_err(|_| TokenError::Malformed);
    let signed = decode(body)?;
    if signed.len() < SIGNATURE_LEN {
        return Err(TokenError::Malformed);
    }
    Ok((signed, decode(footer)?))
}

/// Pre-Authentication Encoding: each piece prefixed by its length, so no two
/// lists of pieces encode the same.
fn pae(pieces: &[&[u8]]) -> Vec<u8> {
    let le64 = |n: usize| ((n as u64) & (u64::MAX >> 1)).to_le_bytes();
    let mut out = le64(pieces.len()).to_vec();
    for piece in pieces {
        out.extend_from_slice(&le64(piece.len()));
        out.extend_from_slice(piece);
    }
    out
}

/// `sub` is a string in PASETO; the id is a number everywhere else.
mod subject {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(id: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        String::deserialize(d)?.parse().map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }

    // The official test vectors, paseto-standard/test-vectors v4.json.
    const SEED: &str = "b4cbfb43df4ce210727d953e4a713307fa19bb7d9f85041438d9e11b942a3774";
    const PUBLIC: &str = "1eb9dbbbbc047c03fd70604e0071f0987e16b28b757225c11f00415d0e20b1a2";
    const PAYLOAD: &[u8] =
        br#"{"data":"this is a signed message","exp":"2022-01-01T00:00:00+00:00"}"#;
    const FOOTER: &[u8] = br#"{"kid":"zVhMiPBP9fRf2snEcT7gFTioeA9COcNy9DfgL1W60haN"}"#;
    const VECTORS: [(&str, &[u8], &[u8], &str); 3] = [
        (
            "4-S-1",
            b"",
            b"",
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9bg_XBBzds8lTZShVlwwKSgeKpLT3yukTw6JUz3W4h_ExsQV-P0V54zemZDcAxFaSeef1QlXEFtkqxT1ciiQEDA",
        ),
        (
            "4-S-2",
            FOOTER,
            b"",
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9v3Jt8mx_TdM2ceTGoqwrh4yDFn0XsHvvV_D0DtwQxVrJEBMl0F2caAdgnpKlt4p7xBnx1HcO-SPo8FPp214HDw.eyJraWQiOiJ6VmhNaVBCUDlmUmYyc25FY1Q3Z0ZUaW9lQTlDT2NOeTlEZmdMMVc2MGhhTiJ9",
        ),
        (
            "4-S-3",
            FOOTER,
            br#"{"test-vector":"4-S-3"}"#,
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9NPWciuD3d0o5eXJXG5pJy-DiVEoyPYWs1YSTwWHNJq6DZD3je5gf-0M4JR9ipdUSJbIovzmBECeaWmaqcaP0DQ.eyJraWQiOiJ6VmhNaVBCUDlmUmYyc25FY1Q3Z0ZUaW9lQTlDT2NOeTlEZmdMMVc2MGhhTiJ9",
        ),
    ];

    #[test]
    fn the_official_test_vectors_sign_and_verify() {
        let (seed, public) = (hex32(SEED), hex32(PUBLIC));
        assert_eq!(public_key(&seed), public);
        for (name, footer, implicit, token) in VECTORS {
            assert_eq!(sign_raw(&seed, PAYLOAD, footer, implicit), token, "{name}");
            let (message, got_footer) = verify_raw(&public, token, implicit).expect(name);
            assert_eq!(message, PAYLOAD, "{name}");
            assert_eq!(got_footer, footer, "{name}");
            // The implicit assertion is signed even though it is not sent.
            assert_eq!(
                verify_raw(&public, token, b"something else"),
                Err(TokenError::BadSignature),
                "{name}"
            );
        }
    }

    fn scout() -> User {
        User {
            id: 7,
            email: "sam@example.com".into(),
            name: "Sam".into(),
            team_number: Some(10101),
            roles: Roles::SCOUT,
        }
    }

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 13, 12, 0, 0).unwrap()
    }

    #[test]
    fn a_token_says_who_on_which_device_until_when() {
        let seed = [9u8; 32];
        let claims = Claims::new(&scout(), "tablet-1", noon());
        let token = sign(&seed, &claims);
        assert!(token.starts_with(HEADER));
        assert_eq!(
            verify(&public_key(&seed), &token, noon()),
            Ok(claims.clone())
        );
        assert_eq!(read_unverified(&token), Ok(claims.clone()));
        assert_eq!(claims.expires_at, noon() + TimeDelta::hours(72));

        // What a device's script reads, without a PASETO library.
        let body = URL_SAFE_NO_PAD.decode(&token[HEADER.len()..]).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body[..body.len() - 64]).unwrap();
        assert_eq!(json["sub"], "7");
        assert_eq!(json["dev"], "tablet-1");
        assert_eq!(json["exp"], "2026-03-16T12:00:00Z");
    }

    #[test]
    fn an_expired_forged_or_altered_token_is_refused() {
        let seed = [9u8; 32];
        let public = public_key(&seed);
        let token = sign(&seed, &Claims::new(&scout(), "tablet-1", noon()));

        assert_eq!(
            verify(&public, &token, noon() + TOKEN_DURATION),
            Err(TokenError::Expired)
        );
        assert!(
            verify(
                &public,
                &token,
                noon() + TOKEN_DURATION - TimeDelta::seconds(1)
            )
            .is_ok()
        );

        // Signed by another key, as a different Pi would.
        let forged = sign(&[1u8; 32], &Claims::new(&scout(), "tablet-1", noon()));
        assert_eq!(
            verify(&public, &forged, noon()),
            Err(TokenError::BadSignature)
        );

        // A lead scout's roles written into a scout's token.
        let mut claims = read_unverified(&token).unwrap();
        claims.roles.is_lead_scout = true;
        let body = URL_SAFE_NO_PAD.decode(&token[HEADER.len()..]).unwrap();
        let mut altered = serde_json::to_vec(&claims).unwrap();
        altered.extend_from_slice(&body[body.len() - 64..]);
        let altered = format!("{HEADER}{}", URL_SAFE_NO_PAD.encode(altered));
        assert_eq!(
            verify(&public, &altered, noon()),
            Err(TokenError::BadSignature)
        );

        for junk in [
            "",
            "v4.public.",
            "v4.local.abc",
            "v3.public.abc",
            "v4.public.!!!",
            "v4.public.YWJj",
        ] {
            assert_eq!(
                verify(&public, junk, noon()),
                Err(TokenError::Malformed),
                "{junk:?}"
            );
        }
    }
}
