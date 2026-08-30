// RFC 7616 MD5 digest auth (the RFC 2069 subset RTSP clients use: no qop, or qop=auth).

#![allow(dead_code)]

use md5::{Digest, Md5};

pub const REALM: &str = "rskycam";

pub fn ha1(username: &str, realm: &str, password: &str) -> String {
    hex_md5(format!("{username}:{realm}:{password}").as_bytes())
}

fn ha2(method: &str, uri: &str) -> String {
    hex_md5(format!("{method}:{uri}").as_bytes())
}

fn hex_md5(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Digest response per RFC 7616 section 3.4.1, `qop=auth` form:
/// `MD5(HA1:nonce:nc:cnonce:qop:HA2)`.
pub fn expected_response(
    ha1: &str,
    nonce: &str,
    nc: &str,
    cnonce: &str,
    qop: &str,
    method: &str,
    uri: &str,
) -> String {
    let ha2 = ha2(method, uri);
    hex_md5(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}").as_bytes())
}

/// The `WWW-Authenticate` challenge header value for a fresh nonce.
pub fn challenge(realm: &str, nonce: &str) -> String {
    format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\"")
}

#[derive(Debug, PartialEq)]
pub struct AuthorizationParams {
    pub username: String,
    pub realm: String,
    pub nonce: String,
    pub uri: String,
    pub response: String,
    pub nc: String,
    pub cnonce: String,
    pub qop: String,
}

/// Parses an `Authorization: Digest ...` header value into its key="value" parts.
pub fn parse_authorization(value: &str) -> Option<AuthorizationParams> {
    let rest = value.strip_prefix("Digest ")?;
    let mut username = None;
    let mut realm = None;
    let mut nonce = None;
    let mut uri = None;
    let mut response = None;
    let mut nc = None;
    let mut cnonce = None;
    let mut qop = None;
    for part in split_params(rest) {
        let (k, v) = part.split_once('=')?;
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "username" => username = Some(v),
            "realm" => realm = Some(v),
            "nonce" => nonce = Some(v),
            "uri" => uri = Some(v),
            "response" => response = Some(v),
            "nc" => nc = Some(v),
            "cnonce" => cnonce = Some(v),
            "qop" => qop = Some(v),
            _ => {}
        }
    }
    Some(AuthorizationParams {
        username: username?,
        realm: realm?,
        nonce: nonce?,
        uri: uri?,
        response: response?,
        nc: nc?,
        cnonce: cnonce?,
        qop: qop?,
    })
}

/// Splits `a="b, c", d=e` into `["a=\"b, c\"", " d=e"]` — a plain `split(',')`
/// would break on the comma inside the quoted `uri`/`qop` value.
fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

pub fn verify(
    params: &AuthorizationParams,
    expected_username: &str,
    expected_ha1: &str,
    expected_nonce: &str,
    method: &str,
) -> bool {
    if params.username != expected_username || params.nonce != expected_nonce {
        return false;
    }
    let want = expected_response(
        expected_ha1,
        &params.nonce,
        &params.nc,
        &params.cnonce,
        &params.qop,
        method,
        &params.uri,
    );
    want == params.response
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 2617 section 3.5 worked example (the MD5 digest RFC that RFC 7616
    // section 5.1 carries forward unchanged for the "MD5" algorithm).
    #[test]
    fn ha1_matches_rfc2617_worked_example() {
        let h = ha1("Mufasa", "testrealm@host.com", "Circle Of Life");
        assert_eq!(h, "939e7578ed9e3c518a452acee763bce9");
    }

    #[test]
    fn round_trip_verifies() {
        let ha1 = ha1("admin", "rskycam", "pa$$word!0");
        let resp = expected_response(
            &ha1,
            "abc123",
            "00000001",
            "xyz",
            "auth",
            "DESCRIBE",
            "rtsp://host/allsky",
        );
        let header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"abc123\", uri=\"rtsp://host/allsky\", response=\"{resp}\", nc=00000001, cnonce=\"xyz\", qop=auth"
        );
        let parsed = parse_authorization(&header).unwrap();
        assert!(verify(&parsed, "admin", &ha1, "abc123", "DESCRIBE"));
        assert!(!verify(
            &parsed,
            "admin",
            &ha1,
            "different-nonce",
            "DESCRIBE"
        ));
    }

    #[test]
    fn rejects_wrong_password() {
        let expected_ha1 = ha1("admin", "rskycam", "pa$$word!0");
        let wrong_ha1 = ha1("admin", "rskycam", "wrong-password");
        let resp = expected_response(&wrong_ha1, "n", "1", "c", "auth", "DESCRIBE", "u");
        let header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"n\", uri=\"u\", response=\"{resp}\", nc=1, cnonce=\"c\", qop=auth"
        );
        let parsed = parse_authorization(&header).unwrap();
        assert!(!verify(&parsed, "admin", &expected_ha1, "n", "DESCRIBE"));
    }
}
