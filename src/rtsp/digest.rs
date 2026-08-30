// RFC 7616 MD5 digest auth (the RFC 2069 subset RTSP clients use: no qop, or qop=auth).

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

/// Digest response in the classic RFC 2069 form: `MD5(HA1:nonce:HA2)`, with
/// no `qop`/`nc`/`cnonce`. live555 -- the RTSP stack VLC 3.x uses -- sends
/// exactly this, so a server that only ever computes the `qop=auth` form can
/// never authenticate such a client no matter how many times it challenges.
pub fn expected_response_rfc2069(ha1: &str, nonce: &str, method: &str, uri: &str) -> String {
    let ha2 = ha2(method, uri);
    hex_md5(format!("{ha1}:{nonce}:{ha2}").as_bytes())
}

/// The `WWW-Authenticate` challenge header value for a fresh nonce.
pub fn challenge(realm: &str, nonce: &str) -> String {
    format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\"")
}

#[derive(Debug, PartialEq)]
pub struct AuthorizationParams {
    pub username: String,
    /// Echoed back by the client from our own challenge; kept so a parsed
    /// header round-trips faithfully, but never checked -- this server only
    /// ever issues one realm, and `expected_ha1` already binds it.
    #[allow(dead_code)]
    pub realm: String,
    pub nonce: String,
    pub uri: String,
    pub response: String,
    /// Absent in the RFC 2069 form (which carries only the five fields
    /// above); present when the client opted into our advertised `qop=auth`.
    pub nc: Option<String>,
    pub cnonce: Option<String>,
    pub qop: Option<String>,
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
        // Not `?`: the RFC 2069 header real RTSP clients send carries none of
        // these, and requiring them turned such a client's Authorization into
        // an unparseable header -- a fresh 401 forever, never authenticating.
        nc,
        cnonce,
        qop,
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
    // We always advertise `qop="auth"`, but the client chooses: live555/VLC
    // ignores it and sends the RFC 2069 five-field header, which is computed
    // over `MD5(HA1:nonce:HA2)` instead.
    let want = match params.qop.as_deref() {
        None | Some("") => {
            expected_response_rfc2069(expected_ha1, &params.nonce, method, &params.uri)
        }
        Some(qop) => expected_response(
            expected_ha1,
            &params.nonce,
            params.nc.as_deref().unwrap_or_default(),
            params.cnonce.as_deref().unwrap_or_default(),
            qop,
            method,
            &params.uri,
        ),
    };
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

    /// live555 (VLC 3.x's RTSP stack) sends the classic five-field RFC 2069
    /// header with no `nc`/`cnonce`/`qop` at all, and computes its response as
    /// `MD5(HA1:nonce:HA2)`. The expected value below is computed by hand from
    /// the RFC 2617 section 3.5 credentials, independently of this module.
    #[test]
    fn accepts_the_rfc2069_no_qop_form_real_clients_send() {
        let ha1 = ha1("Mufasa", "testrealm@host.com", "Circle Of Life");
        assert_eq!(ha1, "939e7578ed9e3c518a452acee763bce9");
        let nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
        // MD5("939e...bce9:dcd9...c093:" + MD5("DESCRIBE:rtsp://host/allsky"))
        let response = "5d8f3f7e744c87b2adde7c7b1ecb66f1";
        let header = format!(
            "Digest username=\"Mufasa\", realm=\"testrealm@host.com\", nonce=\"{nonce}\", uri=\"rtsp://host/allsky\", response=\"{response}\""
        );
        let parsed = parse_authorization(&header).expect("the 5-field form must parse");
        assert_eq!(parsed.qop, None);
        assert_eq!(parsed.nc, None);
        assert_eq!(parsed.cnonce, None);
        assert!(verify(&parsed, "Mufasa", &ha1, nonce, "DESCRIBE"));
        // ...and it is still a real check, not a rubber stamp.
        assert!(!verify(&parsed, "Mufasa", &ha1, nonce, "PLAY")); // wrong method -> wrong HA2
        assert!(!verify(&parsed, "Mufasa", &ha1, "other-nonce", "DESCRIBE"));
        assert!(!verify(&parsed, "Simba", &ha1, nonce, "DESCRIBE"));
    }

    #[test]
    fn rejects_a_wrong_password_in_the_no_qop_form() {
        let expected_ha1 = ha1("admin", "rskycam", "pa$$word!0");
        let wrong_ha1 = ha1("admin", "rskycam", "wrong-password");
        let resp = expected_response_rfc2069(&wrong_ha1, "n", "DESCRIBE", "u");
        let header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"n\", uri=\"u\", response=\"{resp}\""
        );
        let parsed = parse_authorization(&header).unwrap();
        assert!(!verify(&parsed, "admin", &expected_ha1, "n", "DESCRIBE"));
        // The right password in the same form still passes, so the rejection
        // above is about the credential, not about the form being unsupported.
        let right = expected_response_rfc2069(&expected_ha1, "n", "DESCRIBE", "u");
        let ok_header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"n\", uri=\"u\", response=\"{right}\""
        );
        let ok = parse_authorization(&ok_header).unwrap();
        assert!(verify(&ok, "admin", &expected_ha1, "n", "DESCRIBE"));
    }

    /// A client that *does* opt into `qop=auth` must still be verified with
    /// the nc/cnonce-bearing form -- the no-qop branch must not become a way
    /// to bypass it.
    #[test]
    fn qop_form_is_unaffected_by_the_no_qop_branch() {
        let ha1 = ha1("admin", "rskycam", "pa$$word!0");
        let rfc2069 = expected_response_rfc2069(&ha1, "n", "DESCRIBE", "u");
        // Sending the simpler response *while claiming* qop=auth is a mismatch
        // and must be refused.
        let header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"n\", uri=\"u\", response=\"{rfc2069}\", nc=00000001, cnonce=\"c\", qop=auth"
        );
        let parsed = parse_authorization(&header).unwrap();
        assert!(!verify(&parsed, "admin", &ha1, "n", "DESCRIBE"));
    }
}
