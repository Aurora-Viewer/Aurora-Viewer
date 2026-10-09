//! Packet framing: header, zerocoding and appended acks.
//!
//! ```text
//! [flags u8][sequence u32 BE][extra_len u8][extra...][msg number][body...][acks u32 BE...][ack_count u8]
//! ```
//! Zerocoding (`0x00 n` == n zero bytes) applies to everything after the
//! 6-byte header and before the appended acks.

use crate::{DecodeError, Msg, MsgId, Writer};

pub struct PacketFlags;

impl PacketFlags {
    pub const ZEROCODED: u8 = 0x80;
    pub const RELIABLE: u8 = 0x40;
    pub const RESENT: u8 = 0x20;
    pub const ACK: u8 = 0x10;
}

pub const HEADER_SIZE: usize = 6;
pub const MTU: usize = 1200;

#[derive(Debug, Clone)]
pub struct IncomingPacket {
    pub flags: u8,
    pub sequence: u32,
    pub id: MsgId,
    /// Body after the message number, zero-decoded.
    pub body: Vec<u8>,
    pub acks: Vec<u32>,
}

impl IncomingPacket {
    pub fn reliable(&self) -> bool {
        self.flags & PacketFlags::RELIABLE != 0
    }
    pub fn resent(&self) -> bool {
        self.flags & PacketFlags::RESENT != 0
    }
    pub fn decode<M: Msg>(&self) -> Result<M, DecodeError> {
        M::decode(&self.body)
    }
}

fn zero_decode(src: &[u8], out: &mut Vec<u8>) -> Result<(), DecodeError> {
    out.reserve(src.len() * 2);
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b == 0 {
            // zeroCodeExpand: each 0 count byte adds 256 zeros and the run continues.
            i += 1;
            // a trailing lone zero ends the loop: tolerated like LL
            while let Some(&n) = src.get(i) {
                i += 1;
                if n == 0 {
                    out.resize(out.len() + 256, 0);
                } else {
                    out.resize(out.len() + n as usize, 0);
                    break;
                }
                if out.len() > 65536 {
                    return Err(DecodeError::Malformed("zero expansion too large"));
                }
            }
            if out.len() > 65536 {
                return Err(DecodeError::Malformed("zero expansion too large"));
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    Ok(())
}

fn zero_encode(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < src.len() {
        if src[i] == 0 {
            let mut n = 0usize;
            while i < src.len() && src[i] == 0 && n < 255 {
                n += 1;
                i += 1;
            }
            out.push(0);
            out.push(n as u8);
        } else {
            out.push(src[i]);
            i += 1;
        }
    }
}

/// Parse a raw datagram.
pub fn parse_packet(data: &[u8]) -> Result<IncomingPacket, DecodeError> {
    if data.len() < HEADER_SIZE + 1 {
        return Err(DecodeError::Malformed("packet too short"));
    }
    let flags = data[0];
    let sequence = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    let extra = data[5] as usize;
    let mut end = data.len();
    let mut acks = Vec::new();
    if flags & PacketFlags::ACK != 0 {
        let count = data[end - 1] as usize;
        let needed = count * 4 + 1;
        if end < HEADER_SIZE + needed {
            return Err(DecodeError::Malformed("ack block too large"));
        }
        end -= 1;
        let start = end - count * 4;
        for c in data[start..end].as_chunks::<4>().0 {
            acks.push(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
        }
        end = start;
    }
    let payload_raw = &data[HEADER_SIZE..end];
    let decoded;
    let payload: &[u8] = if flags & PacketFlags::ZEROCODED != 0 {
        let mut v = Vec::new();
        zero_decode(payload_raw, &mut v)?;
        decoded = v;
        &decoded
    } else {
        payload_raw
    };
    // LL reads the message number first and then skips the extra header.
    let (id, idlen) = MsgId::decode(payload)?;
    let start = idlen + extra;
    if payload.len() < start {
        return Err(DecodeError::Malformed("extra header overflow"));
    }
    Ok(IncomingPacket {
        flags,
        sequence,
        id,
        body: payload[start..].to_vec(),
        acks,
    })
}

/// Serialize a message into a datagram. `acks` are appended (as many as fit
/// in the MTU); returns how many acks were appended.
pub fn build_packet<M: Msg>(msg: &M, sequence: u32, reliable: bool, resent: bool, acks: &[u32], out: &mut Vec<u8>) -> usize {
    let mut body = Writer::new();
    M::ID.encode(&mut body.buf);
    msg.encode_body(&mut body);
    build_raw(&body.buf, M::ZEROCODED, sequence, reliable, resent, acks, out)
}

/// Frame an already-encoded message (number + body).
pub fn build_raw(payload: &[u8], zerocoded: bool, sequence: u32, reliable: bool, resent: bool, acks: &[u32], out: &mut Vec<u8>) -> usize {
    out.clear();
    let mut flags = 0u8;
    if reliable {
        flags |= PacketFlags::RELIABLE;
    }
    if resent {
        flags |= PacketFlags::RESENT;
    }
    out.push(flags);
    out.extend_from_slice(&sequence.to_be_bytes());
    out.push(0); // no extra header
    if zerocoded {
        let start = out.len();
        zero_encode(payload, out);
        // Only keep zerocoding if it actually helps.
        if out.len() - start < payload.len() {
            out[0] |= PacketFlags::ZEROCODED;
        } else {
            out.truncate(start);
            out.extend_from_slice(payload);
        }
    } else {
        out.extend_from_slice(payload);
    }
    let room = MTU.saturating_sub(out.len() + 1) / 4;
    let n = acks.len().min(room).min(255);
    if n > 0 {
        for a in &acks[..n] {
            out.extend_from_slice(&a.to_be_bytes());
        }
        out.push(n as u8);
        out[0] |= PacketFlags::ACK;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msgs::*;
    use uuid::Uuid;

    #[test]
    fn roundtrip_with_acks_and_zerocode() {
        let mut m = UseCircuitCode::default();
        m.circuit_code.code = 1234;
        m.circuit_code.session_id = Uuid::from_u128(5);
        m.circuit_code.id = Uuid::from_u128(0);
        let mut buf = Vec::new();
        let n = build_packet(&m, 42, true, false, &[1, 2, 3], &mut buf);
        assert_eq!(n, 3);
        let p = parse_packet(&buf).unwrap();
        assert_eq!(p.sequence, 42);
        assert!(p.reliable());
        assert_eq!(p.acks, vec![1, 2, 3]);
        assert_eq!(p.id, UseCircuitCode::ID);
        let back: UseCircuitCode = p.decode().unwrap();
        assert_eq!(back, m);

        // Zerocoded message type
        let mut o = RequestImage::default();
        o.agent_data.agent_id = Uuid::from_u128(9);
        o.request_image.push(request_image::RequestImage {
            image: Uuid::from_u128(77),
            discard_level: 0,
            download_priority: 1.0,
            packet: 0,
            type_: 0,
        });
        build_packet(&o, 7, false, false, &[], &mut buf);
        assert_eq!(buf[0] & PacketFlags::ZEROCODED != 0, RequestImage::ZEROCODED);
        let p = parse_packet(&buf).unwrap();
        let back: RequestImage = p.decode().unwrap();
        assert_eq!(back, o);
    }

    /// RegionHandshake gates the whole region (the simulator sends terrain
    /// and objects only after our reply): it must decode as framed by a
    /// simulator, zerocoded with appended acks, with or without the
    /// trailing RegionInfo4 block that older simulators omit.
    #[test]
    fn region_handshake_from_a_simulator_decodes() {
        let mut m = RegionHandshake::default();
        m.region_info.region_flags = 0x0400_0000;
        m.region_info.sim_access = 13;
        m.region_info.sim_name = crate::str_field("Yikes");
        m.region_info.water_height = 20.0;
        m.region_info.terrain_detail2 = Uuid::from_u128(0xabc);
        m.region_info.terrain_height_range11 = 60.0;
        m.region_info2.region_id = Uuid::from_u128(0x1234_5678);
        m.region_info3.product_name = crate::str_field("Estate / Full Region");
        m.region_info4.push(region_handshake::RegionInfo4 {
            region_flags_extended: 0x0400_0000,
            region_protocols: 1,
        });
        let mut buf = Vec::new();
        build_packet(&m, 3, true, false, &[1, 2], &mut buf);
        assert_ne!(buf[0] & PacketFlags::ZEROCODED, 0);
        let p = parse_packet(&buf).unwrap();
        assert_eq!((p.id, p.acks.as_slice()), (RegionHandshake::ID, &[1, 2][..]));
        assert_eq!(p.decode::<RegionHandshake>().unwrap(), m);

        // an older simulator: no RegionInfo4 block count at all
        m.region_info4.clear();
        let mut body = Writer::new();
        RegionHandshake::ID.encode(&mut body.buf);
        m.encode_body(&mut body);
        assert_eq!(body.buf.pop(), Some(0));
        build_raw(&body.buf, true, 4, true, true, &[], &mut buf);
        let p = parse_packet(&buf).unwrap();
        assert!(p.resent());
        assert_eq!(p.decode::<RegionHandshake>().unwrap(), m);
    }

    #[test]
    fn garbage_does_not_panic() {
        for len in 0..64usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let _ = parse_packet(&data);
            let mut d2 = data.clone();
            if !d2.is_empty() {
                d2[0] = 0xF0;
            }
            let _ = parse_packet(&d2);
        }
    }
}
