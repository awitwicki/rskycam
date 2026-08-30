// Annex-B H.264 -> RTP (RFC 6184), single-NAL or FU-A, RTP-over-TCP interleaved framing.

#![allow(dead_code)] // Public API used by later tasks (encoder, session handler)

use bytes::{BufMut, Bytes, BytesMut};

const PAYLOAD_TYPE: u8 = 96;
const MAX_PAYLOAD: usize = 1400; // keeps interleaved frames well under a typical MTU

/// Incrementally splits NAL units out of a continuous Annex-B byte stream
/// (ffmpeg stdout arrives in arbitrary-sized chunks, not one write per NAL).
/// A NAL is only known to be complete once the *next* start code is seen
/// after it, so the tail of every `feed` call stays buffered until more
/// data -- or `flush()` at stream end -- resolves it.
#[derive(Default)]
pub struct NalSplitter {
    buf: Vec<u8>,
}

impl NalSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `chunk` and returns every NAL unit (payload only, start code
    /// stripped) that is now known complete.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let starts = start_codes(&self.buf);
        if starts.len() < 2 {
            return Vec::new(); // need a following start code to close out NAL 0
        }
        let mut nals = Vec::with_capacity(starts.len() - 1);
        for w in starts.windows(2) {
            let (_, nal_start) = w[0];
            let (next_start, _) = w[1];
            nals.push(self.buf[nal_start..next_start].to_vec());
        }
        // Keep only the unresolved tail: from the last start code onward.
        let (last_marker_start, _) = *starts.last().unwrap();
        self.buf.drain(0..last_marker_start);
        nals
    }

    /// Call at encoder shutdown: whatever's left in the buffer is one final
    /// complete NAL (nothing more will ever arrive to close it out).
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        let starts = start_codes(&self.buf);
        let (_, nal_start) = *starts.first()?;
        if nal_start >= self.buf.len() {
            return None;
        }
        let nal = self.buf[nal_start..].to_vec();
        self.buf.clear();
        Some(nal)
    }
}

/// Returns `(marker_start, nal_start)` for every start code in `data`.
fn start_codes(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                out.push((i, i + 3));
                i += 3;
                continue;
            }
            if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                out.push((i, i + 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// NAL type is the low 5 bits of the first payload byte (SPS=7, PPS=8, IDR
/// slice=5, non-IDR slice=1) -- see ITU-T H.264 section 7.3.1.
pub fn nal_type(nal: &[u8]) -> u8 {
    nal[0] & 0x1f
}

/// One RTP packet, framed for RTSP interleaved delivery: `$`, channel, u16 length, RTP packet.
pub fn interleave(channel: u8, rtp_packet: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(4 + rtp_packet.len());
    out.put_u8(b'$');
    out.put_u8(channel);
    out.put_u16(rtp_packet.len() as u16);
    out.put_slice(rtp_packet);
    out.freeze()
}

fn rtp_header(seq: u16, timestamp: u32, ssrc: u32, marker: bool) -> BytesMut {
    let mut h = BytesMut::with_capacity(12);
    h.put_u8(0x80); // V=2, P=0, X=0, CC=0
    h.put_u8(PAYLOAD_TYPE | if marker { 0x80 } else { 0 });
    h.put_u16(seq);
    h.put_u32(timestamp);
    h.put_u32(ssrc);
    h
}

/// Packetizes one access unit -- the NAL payloads [`NalSplitter`] produced
/// for one encoded frame (parameter sets plus exactly one VCL NAL; see
/// `src/rtsp/encoder.rs`'s access-unit grouping) -- into RTP packets sharing
/// the same 90kHz timestamp, with the marker bit set on the last packet per
/// RFC 6184 section 5.3. `seq` is the sequence number of the *first* packet
/// produced; the caller advances its own counter by the returned packet count.
pub fn packetize_access_unit(
    nals: &[Vec<u8>],
    mut seq: u16,
    timestamp: u32,
    ssrc: u32,
) -> Vec<Bytes> {
    let nals: Vec<&[u8]> = nals
        .iter()
        .map(Vec::as_slice)
        .filter(|n| !n.is_empty())
        .collect();
    let mut packets = Vec::new();
    for (i, nal) in nals.iter().enumerate() {
        let is_last_nal = i == nals.len() - 1;
        if nal.len() <= MAX_PAYLOAD {
            let mut h = rtp_header(seq, timestamp, ssrc, is_last_nal);
            h.put_slice(nal);
            packets.push(h.freeze());
            seq = seq.wrapping_add(1);
        } else {
            let fu_indicator = (nal[0] & 0xe0) | 28; // FU-A
            let nal_header_type = nal_type(nal);
            let payload = &nal[1..];
            let mut offset = 0;
            let chunk_cap = MAX_PAYLOAD - 2; // FU indicator + FU header
            while offset < payload.len() {
                let end = (offset + chunk_cap).min(payload.len());
                let is_first = offset == 0;
                let is_final_fragment = end == payload.len();
                let mut fu_header = nal_header_type;
                if is_first {
                    fu_header |= 0x80;
                }
                if is_final_fragment {
                    fu_header |= 0x40;
                }
                let marker = is_last_nal && is_final_fragment;
                let mut h = rtp_header(seq, timestamp, ssrc, marker);
                h.put_u8(fu_indicator);
                h.put_u8(fu_header);
                h.put_slice(&payload[offset..end]);
                packets.push(h.freeze());
                seq = seq.wrapping_add(1);
                offset = end;
            }
        }
    }
    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_resolves_3_and_4_byte_start_codes_across_feed_calls() {
        let mut s = NalSplitter::new();
        let part1 = [0, 0, 0, 1, 0x67, 0xAA, 0xBB]; // SPS, 4-byte start code
        let part2 = [0, 0, 1, 0x68, 0xCC, 0, 0, 0, 1, 0x65, 0xDD]; // PPS (3-byte), then IDR start
        assert_eq!(s.feed(&part1), Vec::<Vec<u8>>::new()); // no closing start code yet
        let nals = s.feed(&part2);
        assert_eq!(nals, vec![vec![0x67, 0xAA, 0xBB], vec![0x68, 0xCC]]);
        assert_eq!(s.flush(), Some(vec![0x65, 0xDD]));
    }

    #[test]
    fn small_access_unit_is_single_nal_packets_with_marker_on_last() {
        let nals = vec![vec![0x67, 1, 2, 3], vec![0x65, 4, 5, 6]];
        let packets = packetize_access_unit(&nals, 100, 90000, 0xdead_beef);
        assert_eq!(packets.len(), 2);
        // seq
        assert_eq!(u16::from_be_bytes([packets[0][2], packets[0][3]]), 100);
        assert_eq!(u16::from_be_bytes([packets[1][2], packets[1][3]]), 101);
        // marker bit only on the last packet (payload type byte, top bit)
        assert_eq!(packets[0][1] & 0x80, 0);
        assert_eq!(packets[1][1] & 0x80, 0x80);
        // timestamp shared across the access unit
        let ts0 = u32::from_be_bytes([packets[0][4], packets[0][5], packets[0][6], packets[0][7]]);
        let ts1 = u32::from_be_bytes([packets[1][4], packets[1][5], packets[1][6], packets[1][7]]);
        assert_eq!(ts0, 90000);
        assert_eq!(ts1, 90000);
        // single-NAL payload starts right after the 12-byte header
        assert_eq!(&packets[0][12..], &[0x67, 1, 2, 3]);
    }

    #[test]
    fn oversized_nal_is_fragmented_fu_a_with_start_and_end_bits() {
        let mut big_nal = vec![0x65u8]; // NAL header, type 5 (IDR)
        big_nal.extend(std::iter::repeat_n(0xAB, 3000));
        let packets = packetize_access_unit(&[big_nal], 0, 0, 1);
        assert!(packets.len() > 1);
        // FU indicator byte marks type 28 (FU-A)
        for p in &packets {
            assert_eq!(p[12] & 0x1f, 28);
        }
        // first fragment: start bit set, end bit clear
        assert_eq!(packets[0][13] & 0x80, 0x80);
        assert_eq!(packets[0][13] & 0x40, 0);
        // last fragment: end bit set, start bit clear, and marker bit set (last/only NAL)
        let last = packets.last().unwrap();
        assert_eq!(last[13] & 0x40, 0x40);
        assert_eq!(last[13] & 0x80, 0);
        assert_eq!(last[1] & 0x80, 0x80);
        // reassembled FU-A payload type in every header equals the original NAL type (5)
        assert_eq!(packets[0][13] & 0x1f, 5);
        // sequence numbers are contiguous
        for (i, p) in packets.iter().enumerate() {
            assert_eq!(u16::from_be_bytes([p[2], p[3]]), i as u16);
        }
    }

    #[test]
    fn interleave_frame_has_dollar_channel_and_length_prefix() {
        let rtp = [1u8, 2, 3, 4];
        let framed = interleave(0, &rtp);
        assert_eq!(framed[0], b'$');
        assert_eq!(framed[1], 0);
        assert_eq!(u16::from_be_bytes([framed[2], framed[3]]), 4);
        assert_eq!(&framed[4..], &rtp);
    }
}
