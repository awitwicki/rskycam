// Minimal SDP body for a single H.264 video stream, RFC 6184 style.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

/// Builds the DESCRIBE response body from the encoder's SPS/PPS NALs
/// (payload only, no start code) and the RTP payload type / clock rate this
/// server always uses (96 / 90000).
pub fn build(sps: &[u8], pps: &[u8], path: &str) -> String {
    let sprop = format!("{},{}", BASE64.encode(sps), BASE64.encode(pps));
    let profile_level_id = if sps.len() >= 4 {
        format!("{:02x}{:02x}{:02x}", sps[1], sps[2], sps[3])
    } else {
        "42001e".to_string()
    };
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=rskycam\r\n\
         t=0 0\r\n\
         a=tool:rskycam\r\n\
         a=type:broadcast\r\n\
         a=control:*\r\n\
         m=video 0 RTP/AVP 96\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 packetization-mode=1;profile-level-id={profile_level_id};sprop-parameter-sets={sprop}\r\n\
         a=control:{path}\r\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sdp_with_base64_parameter_sets_and_profile_level_id() {
        let sps = [0x67, 0x42, 0x00, 0x1e, 0xAA];
        let pps = [0x68, 0xCE, 0x3C, 0x80];
        let body = build(&sps, &pps, "streamid=0");
        assert!(body.starts_with("v=0\r\n"));
        assert!(body.contains("m=video 0 RTP/AVP 96\r\n"));
        assert!(body.contains("a=rtpmap:96 H264/90000\r\n"));
        assert!(body.contains("profile-level-id=42001e"));
        let expected_sprop = format!(
            "sprop-parameter-sets={},{}",
            BASE64.encode(sps),
            BASE64.encode(pps)
        );
        assert!(body.contains(&expected_sprop));
        assert!(body.contains("a=control:streamid=0\r\n"));
    }
}
