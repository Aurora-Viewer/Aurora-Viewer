//! Second Life UDP message system: packet framing, zerocoding and typed
//! messages generated from `message_template.msg`.
//!
//! Wire format reference: `indra/llmessage/llpacketbuffer.cpp`,
//! `lltemplatemessagereader.cpp` and `lltemplatemessagebuilder.cpp`.

use glam::{DVec3, Quat, Vec3, Vec4};
use uuid::Uuid;

pub mod packet;

pub use packet::{IncomingPacket, PacketFlags, build_packet, parse_packet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsgId {
    High(u8),
    Medium(u8),
    Low(u16),
}

impl MsgId {
    pub fn encode(self, w: &mut Vec<u8>) {
        match self {
            MsgId::High(n) => w.push(n),
            MsgId::Medium(n) => {
                w.push(0xFF);
                w.push(n);
            }
            MsgId::Low(n) => {
                w.push(0xFF);
                w.push(0xFF);
                w.extend_from_slice(&n.to_be_bytes());
            }
        }
    }

    /// Decode a message number; returns the id and its encoded length.
    pub fn decode(b: &[u8]) -> Result<(MsgId, usize), DecodeError> {
        match b {
            [0xFF, 0xFF, hi, lo, ..] => Ok((MsgId::Low(u16::from_be_bytes([*hi, *lo])), 4)),
            [0xFF, 0xFF, ..] => Err(DecodeError::Eof),
            [0xFF, n, ..] => Ok((MsgId::Medium(*n), 2)),
            [0xFF] => Err(DecodeError::Eof),
            [n, ..] => Ok((MsgId::High(*n), 1)),
            [] => Err(DecodeError::Eof),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MsgInfo {
    pub name: &'static str,
    pub zerocoded: bool,
    pub trusted: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unexpected end of message")]
    Eof,
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
}

/// A typed template message.
pub trait Msg: Sized {
    const NAME: &'static str;
    const ID: MsgId;
    const ZEROCODED: bool;
    const TRUSTED: bool;
    fn encode_body(&self, w: &mut Writer);
    fn decode_body(r: &mut Reader) -> Result<Self, DecodeError>;

    /// Decode from a body slice (after the message number, zero-decoded).
    fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        Self::decode_body(&mut r)
    }
}

static ZEROS: [u8; 64] = [0; 64];

/// Little-endian body reader.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            // Older simulators may omit trailing fields: LL zero-fills them.
            if self.remaining() == 0 && n <= ZEROS.len() {
                return Ok(&ZEROS[..n]);
            }
            return Err(DecodeError::Eof);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    #[inline]
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    #[inline]
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    #[inline]
    pub fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.u8()? as i8)
    }
    #[inline]
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    #[inline]
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.u8()? != 0)
    }
    #[inline]
    pub fn vec3(&mut self) -> Result<Vec3, DecodeError> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    #[inline]
    pub fn dvec3(&mut self) -> Result<DVec3, DecodeError> {
        Ok(DVec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    #[inline]
    pub fn vec4(&mut self) -> Result<Vec4, DecodeError> {
        Ok(Vec4::new(self.f32()?, self.f32()?, self.f32()?, self.f32()?))
    }
    /// LLQuaternion is packed as x,y,z with w reconstructed.
    #[inline]
    pub fn quat(&mut self) -> Result<Quat, DecodeError> {
        let v = self.vec3()?;
        Ok(unpack_quat(v))
    }
    #[inline]
    pub fn uuid(&mut self) -> Result<Uuid, DecodeError> {
        Ok(Uuid::from_bytes(self.arr()?))
    }
    #[inline]
    pub fn ipaddr(&mut self) -> Result<[u8; 4], DecodeError> {
        self.arr::<4>()
    }
    /// Ports are transported in network byte order.
    #[inline]
    pub fn ipport(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    pub fn fixed(&mut self, n: usize) -> Result<Vec<u8>, DecodeError> {
        Ok(self.take(n)?.to_vec())
    }
    pub fn var1(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u8()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub fn var2(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u16()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    /// Variable block count; a missing count at the end of data means zero
    /// blocks (matches `LLTemplateMessageReader::decodeData`).
    pub fn block_count(&mut self) -> Result<usize, DecodeError> {
        if self.remaining() == 0 {
            return Ok(0);
        }
        Ok(self.u8()? as usize)
    }
}

/// Reconstruct w from a packed (x,y,z) unit quaternion.
pub fn unpack_quat(v: Vec3) -> Quat {
    let t = 1.0 - v.length_squared();
    let w = if t > 0.0 { t.sqrt() } else { 0.0 };
    Quat::from_xyzw(v.x, v.y, v.z, w)
}

/// Little-endian body writer.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256),
        }
    }
    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    #[inline]
    pub fn i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }
    #[inline]
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    #[inline]
    pub fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    #[inline]
    pub fn dvec3(&mut self, v: DVec3) {
        self.f64(v.x);
        self.f64(v.y);
        self.f64(v.z);
    }
    #[inline]
    pub fn vec4(&mut self, v: Vec4) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
        self.f32(v.w);
    }
    #[inline]
    pub fn quat(&mut self, q: Quat) {
        let mut q = q.normalize();
        if !q.is_finite() {
            q = Quat::IDENTITY;
        }
        if q.w < 0.0 {
            q = -q;
        }
        self.f32(q.x);
        self.f32(q.y);
        self.f32(q.z);
    }
    #[inline]
    pub fn uuid(&mut self, v: Uuid) {
        self.buf.extend_from_slice(v.as_bytes());
    }
    #[inline]
    pub fn ipaddr(&mut self, v: [u8; 4]) {
        self.buf.extend_from_slice(&v);
    }
    #[inline]
    pub fn ipport(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn fixed(&mut self, v: &[u8], n: usize) {
        let m = v.len().min(n);
        self.buf.extend_from_slice(&v[..m]);
        self.buf.resize(self.buf.len() + (n - m), 0);
    }
    pub fn var1(&mut self, v: &[u8]) {
        let n = v.len().min(255);
        self.buf.push(n as u8);
        self.buf.extend_from_slice(&v[..n]);
    }
    pub fn var2(&mut self, v: &[u8]) {
        let n = v.len().min(65535);
        self.u16(n as u16);
        self.buf.extend_from_slice(&v[..n]);
    }
}

/// Helper: build a NUL-terminated string field as SL expects.
pub fn str_field(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// Helper: decode a (possibly NUL-terminated) string field.
pub fn field_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

#[allow(clippy::all, non_snake_case, dead_code, unused_variables)]
pub mod msgs {
    use crate::MsgId;
    use crate::MsgInfo;
    #[allow(unused_imports)]
    use crate::{DecodeError, Reader, Writer};
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

pub use msgs::message_info;

#[cfg(test)]
mod tests {
    use super::msgs::*;
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(StartPingCheck::ID, MsgId::High(1));
        assert_eq!(PacketAck::ID, MsgId::Low(0xFFFB));
        assert_eq!(UseCircuitCode::ID, MsgId::Low(3));
        const { assert!(ObjectUpdate::ZEROCODED) };
        assert_eq!(message_info(MsgId::High(1)).unwrap().name, "StartPingCheck");
    }

    #[test]
    fn roundtrip_chat() {
        let mut m = ChatFromViewer::default();
        m.agent_data.agent_id = Uuid::from_u128(1);
        m.chat_data.message = str_field("hello");
        m.chat_data.channel = 0;
        m.chat_data.type_ = 1;
        let mut w = Writer::new();
        m.encode_body(&mut w);
        let back = ChatFromViewer::decode(&w.buf).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn quat_roundtrip() {
        let q = Quat::from_rotation_z(1.0);
        let mut w = Writer::new();
        w.quat(q);
        let mut r = Reader::new(&w.buf);
        let b = r.quat().unwrap();
        assert!(q.dot(b).abs() > 0.9999);
    }
}
