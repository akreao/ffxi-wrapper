//! The one-minute signed link that signs the server's owner in to the web
//! service without a password.
//!
//! ```text
//! v1.<exp>.<nonce>.<sig>
//!   exp    Unix seconds, now + 60
//!   nonce  16 random bytes, base64url without padding
//!   sig    base64url( HMAC-SHA256( key = hex-decoded secret, msg = "v1.<exp>.<nonce>" ) )
//! ```
//!
//! The desktop app signs, the web service verifies and remembers each nonce so
//! a link works once. `VECTOR_*` in the tests is the shared test vector.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// How long a link is good for.
pub const LIFETIME: u64 = 60;
/// How far ahead of the verifier's clock an expiry may be (a skewed clock).
pub const SKEW: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret isn't 64 hex characters.
    BadSecret,
    /// Not `v1.<exp>.<nonce>.<sig>`.
    Malformed,
    /// A version other than `v1`.
    Version,
    /// The signature doesn't match.
    Signature,
    /// `exp` is in the past.
    Expired,
    /// `exp` is further ahead than a link lives (a forged or skewed token).
    TooFarAhead,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::BadSecret => "the secret is not 64 hex characters",
            Error::Malformed => "the link is malformed",
            Error::Version => "the link is a version this server doesn't know",
            Error::Signature => "the link's signature is wrong",
            Error::Expired => "the link has expired",
            Error::TooFarAhead => "the link expires too far ahead",
        })
    }
}

impl std::error::Error for Error {}

/// A link that passed [`verify`]: the caller still has to refuse a nonce it has seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub exp: u64,
    pub nonce: String,
}

/// The key bytes of a 64-character hex secret.
pub fn parse_secret(secret: &str) -> Result<[u8; 32], Error> {
    let s = secret.trim();
    if s.len() != 64 || !s.is_ascii() {
        return Err(Error::BadSecret);
    }
    let mut out = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).ok_or(Error::BadSecret)?;
        let lo = (pair[1] as char).to_digit(16).ok_or(Error::BadSecret)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Ok(out)
}

fn mac(key: &[u8; 32], msg: &str) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(msg.as_bytes());
    m
}

/// A link for `now` (Unix seconds) with a random nonce.
pub fn sign(secret: &str, now: u64) -> Result<String, Error> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| Error::Malformed)?;
    sign_with_nonce(secret, now, &nonce)
}

/// [`sign`] with a nonce of the caller's choosing, for the shared test vector.
pub fn sign_with_nonce(secret: &str, now: u64, nonce: &[u8; 16]) -> Result<String, Error> {
    let key = parse_secret(secret)?;
    let head = format!("v1.{}.{}", now + LIFETIME, URL_SAFE_NO_PAD.encode(nonce));
    let sig = mac(&key, &head).finalize().into_bytes();
    Ok(format!("{head}.{}", URL_SAFE_NO_PAD.encode(sig)))
}

/// Check a token against the secret and the clock: version, signature
/// (constant time), and `now <= exp <= now + LIFETIME + SKEW`.
pub fn verify(secret: &str, token: &str, now: u64) -> Result<Verified, Error> {
    let key = parse_secret(secret)?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 4 {
        return Err(Error::Malformed);
    }
    if parts[0] != "v1" {
        return Err(Error::Version);
    }
    let exp: u64 = parts[1].parse().map_err(|_| Error::Malformed)?;
    let sig = URL_SAFE_NO_PAD.decode(parts[3]).map_err(|_| Error::Malformed)?;
    if URL_SAFE_NO_PAD.decode(parts[2]).map(|n| n.len()) != Ok(16) {
        return Err(Error::Malformed);
    }
    let head = format!("v1.{}.{}", parts[1], parts[2]);
    mac(&key, &head).verify_slice(&sig).map_err(|_| Error::Signature)?;
    if exp < now {
        return Err(Error::Expired);
    }
    if exp > now + LIFETIME + SKEW {
        return Err(Error::TooFarAhead);
    }
    Ok(Verified { exp, nonce: parts[2].to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared with the desktop app's signer: the same inputs must give this token.
    const VECTOR_SECRET: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const VECTOR_NOW: u64 = 1_800_000_000;
    const VECTOR_NONCE: [u8; 16] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    ];

    fn token() -> String {
        sign_with_nonce(VECTOR_SECRET, VECTOR_NOW, &VECTOR_NONCE).unwrap()
    }

    #[test]
    fn shared_vector() {
        assert_eq!(token(), SIGNED, "the token changed: the desktop app's signer must match");
    }

    const SIGNED: &str = include_str!("owner_link_vector.txt");

    #[test]
    fn verifies_and_returns_the_nonce() {
        let v = verify(VECTOR_SECRET, &token(), VECTOR_NOW + 10).unwrap();
        assert_eq!(v.exp, VECTOR_NOW + 60);
        assert_eq!(v.nonce, "AAECAwQFBgcICQoLDA0ODw");
    }

    #[test]
    fn expired_future_and_tampered_fail() {
        let t = token();
        assert_eq!(verify(VECTOR_SECRET, &t, VECTOR_NOW + 61), Err(Error::Expired));
        // Signed for a time more than a link lives (plus skew) ahead of the verifier.
        assert_eq!(verify(VECTOR_SECRET, &t, VECTOR_NOW - 6), Err(Error::TooFarAhead));
        assert!(verify(VECTOR_SECRET, &t, VECTOR_NOW - 5).is_ok());
        let mut tampered: Vec<&str> = t.split('.').collect();
        tampered[1] = "1800009999";
        assert_eq!(verify(VECTOR_SECRET, &tampered.join("."), VECTOR_NOW), Err(Error::Signature));
        let other = "ff".repeat(32);
        assert_eq!(verify(&other, &t, VECTOR_NOW), Err(Error::Signature));
    }

    #[test]
    fn wrong_version_and_junk_fail() {
        let t = token().replacen("v1", "v2", 1);
        assert_eq!(verify(VECTOR_SECRET, &t, VECTOR_NOW), Err(Error::Version));
        assert_eq!(verify(VECTOR_SECRET, "v1.1.2", VECTOR_NOW), Err(Error::Malformed));
        assert_eq!(verify(VECTOR_SECRET, "v1.x.AAECAwQFBgcICQoLDA0ODw.AAAA", VECTOR_NOW), Err(Error::Malformed));
        assert_eq!(verify(VECTOR_SECRET, "", VECTOR_NOW), Err(Error::Malformed));
    }

    #[test]
    fn secrets_must_be_64_hex_characters() {
        assert_eq!(parse_secret(""), Err(Error::BadSecret));
        assert_eq!(parse_secret(&"a".repeat(63)), Err(Error::BadSecret));
        assert_eq!(parse_secret(&"g".repeat(64)), Err(Error::BadSecret));
        assert!(parse_secret(&"AB".repeat(32)).is_ok());
        assert_eq!(sign("short", 1), Err(Error::BadSecret));
    }

    #[test]
    fn random_nonces_differ() {
        let a = sign(VECTOR_SECRET, VECTOR_NOW).unwrap();
        let b = sign(VECTOR_SECRET, VECTOR_NOW).unwrap();
        assert_ne!(a, b);
        assert!(verify(VECTOR_SECRET, &a, VECTOR_NOW).is_ok());
    }
}
