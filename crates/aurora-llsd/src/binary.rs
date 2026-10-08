//! LLSD binary format (`application/llsd+binary`), as used by mesh asset headers.

use crate::{Llsd, LlsdError, Map};
use uuid::Uuid;

const MAX_DEPTH: usize = 128;
const HEADER: &[u8] = b"<? LLSD/Binary ?>";

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LlsdError> {
        let end = self.pos.checked_add(n).ok_or(LlsdError::Eof)?;
        if end > self.data.len() {
            return Err(LlsdError::Eof);
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, LlsdError> {
        Ok(self.take(1)?[0])
    }

    fn u32_be(&mut self) -> Result<u32, LlsdError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn len(&mut self) -> Result<usize, LlsdError> {
        let n = self.u32_be()? as usize;
        if n > self.data.len().saturating_sub(self.pos) {
            return Err(LlsdError::Eof);
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, LlsdError> {
        let n = self.len()?;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn value(&mut self, depth: usize) -> Result<Llsd, LlsdError> {
        if depth > MAX_DEPTH {
            return Err(LlsdError::Invalid("nesting too deep"));
        }
        let t = self.u8()?;
        Ok(match t {
            b'!' => Llsd::Undef,
            b'1' => Llsd::Boolean(true),
            b'0' => Llsd::Boolean(false),
            b'i' => Llsd::Integer(self.u32_be()? as i32),
            b'r' => {
                let b = self.take(8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                Llsd::Real(f64::from_be_bytes(a))
            }
            b'd' => {
                let b = self.take(8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                Llsd::Date(f64::from_le_bytes(a))
            }
            b'u' => Llsd::Uuid(Uuid::from_slice(self.take(16)?).unwrap_or(Uuid::nil())),
            b's' => Llsd::String(self.string()?),
            b'l' => Llsd::Uri(self.string()?),
            b'b' => {
                let n = self.len()?;
                Llsd::Binary(self.take(n)?.to_vec())
            }
            b'[' => {
                let n = self.u32_be()? as usize;
                let mut a = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    a.push(self.value(depth + 1)?);
                }
                if self.u8()? != b']' {
                    return Err(LlsdError::Invalid("missing ']'"));
                }
                Llsd::Array(a)
            }
            b'{' => {
                let n = self.u32_be()? as usize;
                let mut m = Map::new();
                for _ in 0..n {
                    let k = self.u8()?;
                    if k != b'k' {
                        return Err(LlsdError::Invalid("expected map key"));
                    }
                    let key = self.string()?;
                    let v = self.value(depth + 1)?;
                    m.insert(key, v);
                }
                if self.u8()? != b'}' {
                    return Err(LlsdError::Invalid("missing '}'"));
                }
                Llsd::Map(m)
            }
            _ => return Err(LlsdError::Invalid("unknown binary type tag")),
        })
    }
}

/// Parse binary LLSD; returns the value and the number of bytes consumed.
pub fn from_binary(data: &[u8]) -> Result<(Llsd, usize), LlsdError> {
    let mut r = Reader { data, pos: 0 };
    if data.starts_with(HEADER) {
        r.pos = HEADER.len();
        while r.pos < data.len() && data[r.pos].is_ascii_whitespace() {
            r.pos += 1;
        }
    }
    let v = r.value(0)?;
    Ok((v, r.pos))
}

fn write(v: &Llsd, out: &mut Vec<u8>) {
    match v {
        Llsd::Undef => out.push(b'!'),
        Llsd::Boolean(b) => out.push(if *b { b'1' } else { b'0' }),
        Llsd::Integer(i) => {
            out.push(b'i');
            out.extend_from_slice(&i.to_be_bytes());
        }
        Llsd::Real(r) => {
            out.push(b'r');
            out.extend_from_slice(&r.to_be_bytes());
        }
        Llsd::Date(d) => {
            out.push(b'd');
            out.extend_from_slice(&d.to_le_bytes());
        }
        Llsd::Uuid(u) => {
            out.push(b'u');
            out.extend_from_slice(u.as_bytes());
        }
        Llsd::String(s) => {
            out.push(b's');
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Llsd::Uri(s) => {
            out.push(b'l');
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Llsd::Binary(b) => {
            out.push(b'b');
            out.extend_from_slice(&(b.len() as u32).to_be_bytes());
            out.extend_from_slice(b);
        }
        Llsd::Array(a) => {
            out.push(b'[');
            out.extend_from_slice(&(a.len() as u32).to_be_bytes());
            for e in a {
                write(e, out);
            }
            out.push(b']');
        }
        Llsd::Map(m) => {
            out.push(b'{');
            out.extend_from_slice(&(m.len() as u32).to_be_bytes());
            for (k, e) in m {
                out.push(b'k');
                out.extend_from_slice(&(k.len() as u32).to_be_bytes());
                out.extend_from_slice(k.as_bytes());
                write(e, out);
            }
            out.push(b'}');
        }
    }
}

pub fn to_binary(v: &Llsd) -> Vec<u8> {
    let mut out = Vec::new();
    write(v, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llsd_map;

    #[test]
    fn roundtrip() {
        let v = llsd_map! { "high_lod" => llsd_map!{"offset" => 0, "size" => 1234}, "x" => 1.5, "u" => Uuid::from_u128(7) };
        let b = to_binary(&v);
        let (back, used) = from_binary(&b).unwrap();
        assert_eq!(used, b.len());
        assert_eq!(v, back);
    }

    #[test]
    fn truncated_is_error() {
        let b = to_binary(&llsd_map! {"a" => "hello"});
        for n in 0..b.len() {
            assert!(from_binary(&b[..n]).is_err());
        }
    }
}
