//! Second login factor: 6-digit codes from an authenticator app (RFC 6238 TOTP,
//! HMAC-SHA1, 30-second steps), the same scheme Google Authenticator and others use.

use hmac::{Hmac, Mac};
use sha1::Sha1;

const STEP_SECS: u64 = 30;
const DIGITS: u32 = 6;
/// Codes from one step before or after are accepted too: phone clocks drift.
const SKEW_STEPS: u64 = 1;
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new random secret in base32, the form authenticator apps take.
pub fn new_secret() -> String {
    let mut bytes = [0u8; 20];
    getrandom::getrandom(&mut bytes).expect("OS random generator");
    base32_encode(&bytes)
}

/// The link an authenticator app reads from a QR code.
pub fn otpauth_url(secret: &str, account: &str) -> String {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect()
    };
    format!("otpauth://totp/{}:{}?secret={secret}&issuer={}&digits={DIGITS}&period={STEP_SECS}", enc("Облако"), enc(account), enc("Облако"))
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The step the code belongs to, if it is valid at `now` (unix seconds).
/// Callers refuse a step they have already accepted, so a code can't be used twice.
pub fn verify(secret: &str, code: &str, now: u64) -> Option<u64> {
    let code = code.trim().replace(' ', "");
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let key = base32_decode(secret)?;
    let current = now / STEP_SECS;
    (current.saturating_sub(SKEW_STEPS)..=current + SKEW_STEPS).find(|&step| constant_eq(&hotp(&key, step), &code))
}

fn hotp(key: &[u8], counter: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([digest[offset] & 0x7f, digest[offset + 1], digest[offset + 2], digest[offset + 3]]);
    format!("{:0width$}", bin % 10u32.pow(DIGITS), width = DIGITS as usize)
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Accepts lower case, spaces and padding, as people type secrets by hand.
fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let v = BASE32.iter().position(|&x| x as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B, SHA-1 key "12345678901234567890" (last 6 of the 8 digits).
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn rfc_vectors() {
        let key = base32_decode(RFC_SECRET).unwrap();
        assert_eq!(key, b"12345678901234567890");
        assert_eq!(hotp(&key, 59 / 30), "287082");
        assert_eq!(hotp(&key, 1_111_111_109 / 30), "081804");
        assert_eq!(hotp(&key, 2_000_000_000 / 30), "279037");
    }

    #[test]
    fn verify_accepts_neighbour_steps_only() {
        let now = 1_111_111_109;
        assert_eq!(verify(RFC_SECRET, "081804", now), Some(now / 30));
        assert_eq!(verify(RFC_SECRET, "081 804", now + 30), Some(now / 30));
        assert_eq!(verify(RFC_SECRET, "081804", now + 90), None);
        assert_eq!(verify(RFC_SECRET, "000000", now), None);
        assert_eq!(verify(RFC_SECRET, "08180", now), None);
        assert_eq!(verify(RFC_SECRET, "abcdef", now), None);
    }

    #[test]
    fn base32_round_trip() {
        let s = new_secret();
        assert_eq!(s.len(), 32);
        assert_eq!(base32_encode(&base32_decode(&s).unwrap()), s);
        assert_eq!(base32_decode("gezd gnbv"), base32_decode("GEZDGNBV"));
        assert!(base32_decode("1!").is_none());
    }

    #[test]
    fn otpauth_url_is_escaped() {
        let url = otpauth_url("ABC", "me@home");
        assert!(url.starts_with("otpauth://totp/%D0%9E"), "{url}");
        assert!(url.contains(":me%40home?secret=ABC&issuer="), "{url}");
    }
}
