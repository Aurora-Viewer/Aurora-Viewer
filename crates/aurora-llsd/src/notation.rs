//! LLSD notation format (`application/llsd+notation`), as carried by the
//! GLTF material override payload of `GenericStreamingMessage`.
//!
//! Port of `LLSDNotationParser` (llcommon/llsdserialize.cpp, originally
//! LGPL 2.1): same tokens, delimiters and string escapes. Parsing stops after
//! the first value; trailing bytes (the sender's NUL) are ignored.

use crate::{Llsd, LlsdError, Map};
use uuid::Uuid;

const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn next(&mut self) -> Result<u8, LlsdError> {
        let c = self.peek().ok_or(LlsdError::Eof)?;
        self.pos += 1;
        Ok(c)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LlsdError> {
        let end = self.pos.checked_add(n).ok_or(LlsdError::Eof)?;
        let s = self.data.get(self.pos..end).ok_or(LlsdError::Eof)?;
        self.pos = end;
        Ok(s)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// Bytes while `f` holds (number and word tokens).
    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(&f) {
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    fn value(&mut self, depth: usize) -> Result<Llsd, LlsdError> {
        if depth == 0 {
            return Err(LlsdError::Invalid("notation nested too deeply"));
        }
        self.skip_space();
        let c = self.peek().ok_or(LlsdError::Eof)?;
        Ok(match c {
            b'{' => self.map(depth - 1)?,
            b'[' => self.array(depth - 1)?,
            b'!' => {
                self.pos += 1;
                Llsd::Undef
            }
            b'0' => {
                self.pos += 1;
                Llsd::Boolean(false)
            }
            b'1' => {
                self.pos += 1;
                Llsd::Boolean(true)
            }
            b'f' | b'F' | b't' | b'T' => {
                // single letter, or the whole word (false / FALSE / true / TRUE)
                let word = self.take_while(|c| c.is_ascii_alphabetic());
                match word {
                    b"f" | b"F" | b"false" | b"FALSE" => Llsd::Boolean(false),
                    b"t" | b"T" | b"true" | b"TRUE" => Llsd::Boolean(true),
                    _ => return Err(LlsdError::Invalid("notation boolean")),
                }
            }
            b'i' => {
                self.pos += 1;
                let s = self.take_while(|c| c.is_ascii_digit() || c == b'-' || c == b'+');
                let v = std::str::from_utf8(s)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .ok_or(LlsdError::Invalid("notation integer"))?;
                // istream >> S32 fails (and LL drops the value) out of range
                Llsd::Integer(i32::try_from(v).map_err(|_| LlsdError::Invalid("notation integer"))?)
            }
            b'r' => {
                self.pos += 1;
                let s = self.take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.'));
                let v = std::str::from_utf8(s)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or(LlsdError::Invalid("notation real"))?;
                Llsd::Real(v)
            }
            b'u' => {
                self.pos += 1;
                let s = self.take(36)?;
                let id = std::str::from_utf8(s)
                    .ok()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or(LlsdError::Invalid("notation uuid"))?;
                Llsd::Uuid(id)
            }
            b'"' | b'\'' | b's' => Llsd::String(self.string()?),
            b'l' => {
                self.pos += 1;
                let d = self.next()?;
                Llsd::Uri(self.delimited(d)?)
            }
            b'd' => {
                self.pos += 1;
                let d = self.next()?;
                Llsd::Date(crate::xml_llsd::parse_date(&self.delimited(d)?))
            }
            b'b' => Llsd::Binary(self.binary()?),
            _ => return Err(LlsdError::Invalid("notation: unexpected character")),
        })
    }

    fn map(&mut self, depth: usize) -> Result<Llsd, LlsdError> {
        self.next()?; // '{'
        let mut m = Map::new();
        loop {
            // keys and values may be separated by spaces, ':' and ','
            while self.peek().is_some_and(|c| c.is_ascii_whitespace() || c == b',') {
                self.pos += 1;
            }
            match self.peek().ok_or(LlsdError::Eof)? {
                b'}' => {
                    self.pos += 1;
                    return Ok(Llsd::Map(m));
                }
                b'"' | b'\'' | b's' => {
                    let key = self.string()?;
                    while self.peek().is_some_and(|c| c.is_ascii_whitespace() || c == b':') {
                        self.pos += 1;
                    }
                    let v = self.value(depth)?;
                    m.insert(key, v);
                }
                _ => return Err(LlsdError::Invalid("notation map key")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Llsd, LlsdError> {
        self.next()?; // '['
        let mut a = Vec::new();
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace() || c == b',') {
                self.pos += 1;
            }
            if self.peek().ok_or(LlsdError::Eof)? == b']' {
                self.pos += 1;
                return Ok(Llsd::Array(a));
            }
            a.push(self.value(depth)?);
        }
    }

    /// `"..."`, `'...'` (escaped) or `s(len)"raw"` (deserialize_string).
    fn string(&mut self) -> Result<String, LlsdError> {
        match self.next()? {
            d @ (b'"' | b'\'') => self.delimited(d),
            b's' => {
                let raw = self.sized()?;
                Ok(String::from_utf8_lossy(raw).into_owned())
            }
            _ => Err(LlsdError::Invalid("notation string")),
        }
    }

    /// `(len)"raw bytes"` after the type letter.
    fn sized(&mut self) -> Result<&'a [u8], LlsdError> {
        if self.next()? != b'(' {
            return Err(LlsdError::Invalid("notation sized value"));
        }
        let n = self.take_while(|c| c != b')');
        let len: usize = std::str::from_utf8(n)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or(LlsdError::Invalid("notation size"))?;
        self.next()?; // ')'
        if !matches!(self.next()?, b'"' | b'\'') {
            return Err(LlsdError::Invalid("notation sized value"));
        }
        let raw = self.take(len)?;
        if !matches!(self.next()?, b'"' | b'\'') {
            return Err(LlsdError::Invalid("notation sized value"));
        }
        Ok(raw)
    }

    /// Escaped string up to `delim` (deserialize_string_delim).
    fn delimited(&mut self, delim: u8) -> Result<String, LlsdError> {
        let mut out = Vec::new();
        loop {
            let c = self.next()?;
            if c == delim {
                break;
            }
            if c != b'\\' {
                out.push(c);
                continue;
            }
            let e = self.next()?;
            out.push(match e {
                b'a' => 0x07,
                b'b' => 0x08,
                b'f' => 0x0C,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => 0x0B,
                b'x' => {
                    let hi = hex_nybble(self.next()?);
                    let lo = hex_nybble(self.next()?);
                    (hi << 4) | lo
                }
                other => other,
            });
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// `b(len)"raw"`, `b16"hex"` or `b64"base64"` (parseBinary).
    fn binary(&mut self) -> Result<Vec<u8>, LlsdError> {
        self.next()?; // 'b'
        match self.peek().ok_or(LlsdError::Eof)? {
            b'(' => Ok(self.sized()?.to_vec()),
            b'1' => {
                if self.take(3)? != b"16\"" {
                    return Err(LlsdError::Invalid("notation binary"));
                }
                let hex = self.take_while(|c| c != b'"');
                self.next()?;
                Ok(hex
                    .chunks(2)
                    .map(|p| (hex_nybble(p[0]) << 4) | p.get(1).map_or(0, |&c| hex_nybble(c)))
                    .collect())
            }
            b'6' => {
                if self.take(3)? != b"64\"" {
                    return Err(LlsdError::Invalid("notation binary"));
                }
                let text = self.take_while(|c| c != b'"');
                self.next()?;
                base64_decode(text).ok_or(LlsdError::Invalid("notation base64"))
            }
            _ => Err(LlsdError::Invalid("notation binary")),
        }
    }
}

/// hex_as_nybble: anything else counts as 0.
fn hex_nybble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Parse the first LLSD notation value of `data`.
pub fn from_notation(data: &[u8]) -> Result<Llsd, LlsdError> {
    Parser { data, pos: 0 }.value(MAX_DEPTH)
}

/// One-line LLSD notation of a value, like LLSDNotationFormatter without
/// pretty printing (map keys and strings single-quoted with the
/// serialize_string escapes, `r` reals, `d"…"` dates, `b64"…"` binaries).
pub fn to_notation(v: &Llsd) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_string(s: &str, out: &mut String) {
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
}

fn write_value(v: &Llsd, out: &mut String) {
    use base64::Engine;
    match v {
        Llsd::Undef => out.push('!'),
        Llsd::Boolean(b) => out.push(if *b { '1' } else { '0' }),
        Llsd::Integer(i) => out.push_str(&format!("i{i}")),
        Llsd::Real(r) => out.push_str(&format!("r{r}")),
        Llsd::String(s) => write_string(s, out),
        Llsd::Uuid(u) => out.push_str(&format!("u{u}")),
        Llsd::Date(d) => out.push_str(&format!("d\"{}\"", crate::xml_llsd::format_date(*d))),
        Llsd::Uri(u) => {
            out.push('l');
            write_string(u, out);
        }
        Llsd::Binary(b) => out.push_str(&format!("b64\"{}\"", base64::engine::general_purpose::STANDARD.encode(b))),
        Llsd::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(x, out);
            }
            out.push(']');
        }
        Llsd::Map(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(x, out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gltf_override_payload() {
        let msg = b"{'id':i305419896,'te':[i0,i2],'od':[{'bc':[r1,r0.5,r0.5,r1],'tex':[!,!,!,uffffffff-ffff-ffff-ffff-ffffffffffff],'ti':[{'s':[r2,r2]}]},{'mf':r0,'ds':1}]}\0";
        let v = from_notation(msg).unwrap();
        assert_eq!(v.get("id").as_i32(), 305419896);
        assert_eq!(v.get("te").as_array().len(), 2);
        let od = v.get("od");
        assert_eq!(od.at(0).get("bc").at(1), &Llsd::Real(0.5));
        assert!(od.at(0).get("tex").at(0).is_undef());
        assert_eq!(od.at(0).get("tex").at(3), &Llsd::Uuid(Uuid::from_u128(u128::MAX)));
        assert_eq!(od.at(0).get("ti").at(0).get("s").at(1), &Llsd::Real(2.0));
        assert_eq!(od.at(1).get("mf"), &Llsd::Real(0.0));
        assert_eq!(od.at(1).get("ds"), &Llsd::Boolean(true));
    }

    #[test]
    fn writes_what_it_reads() {
        // a teleport history line (LLTeleportHistoryPersistentItem::toLLSD)
        let mut m = Map::new();
        m.insert("title".into(), Llsd::String("Place d'Aurora, Aurora \\ Démo\n".into()));
        m.insert("global_pos".into(), Llsd::Array(vec![Llsd::Real(256140.5), Llsd::Real(256120.0), Llsd::Real(25.0)]));
        m.insert("date".into(), Llsd::Date(1_760_000_000.25));
        m.insert("slurl".into(), Llsd::String(String::new()));
        m.insert("n".into(), Llsd::Array(vec![Llsd::Undef, Llsd::Boolean(true), Llsd::Integer(-3)]));
        m.insert("id".into(), Llsd::Uuid(Uuid::from_u128(7)));
        m.insert("bin".into(), Llsd::Binary(vec![0, 255, 7]));
        let v = Llsd::Map(m);
        let text = to_notation(&v);
        assert!(!text.contains('\n'), "one line: {text}");
        assert_eq!(from_notation(text.as_bytes()).unwrap(), v);
    }

    #[test]
    fn scalars_and_strings() {
        assert_eq!(from_notation(b" i-42").unwrap(), Llsd::Integer(-42));
        assert_eq!(from_notation(b"r-1.5e2").unwrap(), Llsd::Real(-150.0));
        assert_eq!(from_notation(b"true").unwrap(), Llsd::Boolean(true));
        assert_eq!(from_notation(b"F").unwrap(), Llsd::Boolean(false));
        assert_eq!(from_notation(b"\"a\\\"b\\x41\\n\"").unwrap(), Llsd::String("a\"bA\n".into()));
        assert_eq!(from_notation(b"s(3)\"x'y\"").unwrap(), Llsd::String("x'y".into()));
        assert_eq!(from_notation(b"b(2)\"\x01\x02\"").unwrap(), Llsd::Binary(vec![1, 2]));
        assert_eq!(from_notation(b"b16\"0aFF\"").unwrap(), Llsd::Binary(vec![10, 255]));
        assert_eq!(from_notation(b"b64\"AQID\"").unwrap(), Llsd::Binary(vec![1, 2, 3]));
        assert_eq!(
            from_notation(b"{ \"k\" : [ ] , s(1)\"z\":! }").unwrap().get("k"),
            &Llsd::Array(vec![])
        );
    }

    #[test]
    fn malformed_is_an_error() {
        assert!(from_notation(b"{'a':").is_err());
        assert!(from_notation(b"[i1,").is_err());
        assert!(from_notation(b"i99999999999").is_err());
        assert!(from_notation(b"s(100)\"short\"").is_err());
        assert!(from_notation(b"?").is_err());
    }
}
