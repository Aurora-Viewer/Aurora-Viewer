//! LLSD binary format (`application/llsd+binary`), as used by mesh asset headers.

use crate::{Budget, Llsd, LlsdError, MAP_ENTRY_COST, Map, VALUE_COST};
use uuid::Uuid;

const MAX_DEPTH: usize = 128;
const HEADER: &[u8] = b"<? LLSD/Binary ?>";

/// Smallest encoding of one map entry: `k`, the key length and a one-byte value.
const MIN_MAP_ENTRY_BYTES: usize = 6;

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    budget: Budget,
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

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn len(&mut self) -> Result<usize, LlsdError> {
        let n = self.u32_be()? as usize;
        if n > self.remaining() {
            return Err(LlsdError::Eof);
        }
        Ok(n)
    }

    /// A count of items that each take at least `min_bytes`, followed by a
    /// one-byte closing tag: a count the rest of the input cannot hold is
    /// refused before anything is reserved for it.
    fn count(&mut self, min_bytes: usize) -> Result<usize, LlsdError> {
        let n = self.u32_be()? as usize;
        let needed = n.checked_mul(min_bytes).and_then(|b| b.checked_add(1)).ok_or(LlsdError::Eof)?;
        if needed > self.remaining() {
            return Err(LlsdError::Eof);
        }
        Ok(n)
    }

    /// A length-prefixed payload, charged to the budget before it is copied.
    fn payload(&mut self) -> Result<&'a [u8], LlsdError> {
        let n = self.len()?;
        self.budget.charge(n)?;
        self.take(n)
    }

    fn string(&mut self) -> Result<String, LlsdError> {
        let raw = self.payload()?;
        let s = String::from_utf8_lossy(raw).into_owned();
        // invalid bytes grow into U+FFFD
        self.budget.charge(s.len().saturating_sub(raw.len()))?;
        Ok(s)
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
            b'b' => Llsd::Binary(self.payload()?.to_vec()),
            b'[' => {
                // every element takes at least one byte; the slots are
                // charged here, the elements only for their own heap
                let n = self.count(1)?;
                self.budget.charge_values(n)?;
                let mut a = Vec::with_capacity(n);
                for _ in 0..n {
                    a.push(self.value(depth + 1)?);
                }
                if self.u8()? != b']' {
                    return Err(LlsdError::Invalid("missing ']'"));
                }
                Llsd::Array(a)
            }
            b'{' => {
                let n = self.count(MIN_MAP_ENTRY_BYTES)?;
                let mut m = Map::new();
                for _ in 0..n {
                    let k = self.u8()?;
                    if k != b'k' {
                        return Err(LlsdError::Invalid("expected map key"));
                    }
                    self.budget.charge(MAP_ENTRY_COST)?;
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
///
/// The tree may hold at most a few times the input in estimated heap (and
/// never more than [`crate::MAX_PARSED_BYTES`]): a value in memory can be
/// far larger than its encoding.
/// Past that, the parse fails with [`LlsdError::TooLarge`].
pub fn from_binary(data: &[u8]) -> Result<(Llsd, usize), LlsdError> {
    parse(data, Budget::for_input(data.len())).map(|(v, used, _)| (v, used))
}

/// [`from_binary`] for files the viewer wrote itself (the inventory cache,
/// the saved environment): the same per-byte rule, without the fixed cap,
/// which a large inventory exceeds.
pub fn from_binary_local(data: &[u8]) -> Result<(Llsd, usize), LlsdError> {
    parse(data, Budget::for_local_input(data.len())).map(|(v, used, _)| (v, used))
}

/// The value, the bytes consumed and the budget left.
fn parse(data: &[u8], budget: Budget) -> Result<(Llsd, usize, Budget), LlsdError> {
    let mut r = Reader { data, pos: 0, budget };
    r.budget.charge(VALUE_COST)?;
    if data.starts_with(HEADER) {
        r.pos = HEADER.len();
        while r.pos < data.len() && data[r.pos].is_ascii_whitespace() {
            r.pos += 1;
        }
    }
    let v = r.value(0)?;
    Ok((v, r.pos, r.budget))
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

    /// `[`, a count, `count` one-byte values and `]`.
    fn array_of(count: usize, value: &[u8]) -> Vec<u8> {
        let mut b = vec![b'['];
        b.extend_from_slice(&(count as u32).to_be_bytes());
        for _ in 0..count {
            b.extend_from_slice(value);
        }
        b.push(b']');
        b
    }

    #[test]
    fn the_tree_stays_within_its_budget() {
        // 32 bytes of tree per input byte: a few thousand still parse...
        let (v, _) = from_binary(&array_of(1000, b"!")).unwrap();
        assert_eq!(v.len(), 1000);
        // ...a megabyte of them is twice the per-byte allowance
        assert!(matches!(from_binary(&array_of(1 << 20, b"!")), Err(LlsdError::TooLarge)));
        assert!(matches!(from_binary(&array_of(1 << 20, b"1")), Err(LlsdError::TooLarge)));
    }

    #[test]
    fn counts_the_input_cannot_hold_are_refused() {
        // an array of 2^32-1 items in 9 bytes: refused before any reservation
        let mut b = vec![b'['];
        b.extend_from_slice(&u32::MAX.to_be_bytes());
        b.extend_from_slice(b"!!!]");
        assert!(matches!(from_binary(&b), Err(LlsdError::Eof)));
        // each map entry takes at least 6 bytes
        let mut m = vec![b'{'];
        m.extend_from_slice(&3u32.to_be_bytes());
        m.extend_from_slice(b"k\0\0\0\0!k\0\0\0\0!}");
        assert!(matches!(from_binary(&m), Err(LlsdError::Eof)));
    }

    #[test]
    fn real_shaped_data_parses_within_budget() {
        // a mesh-like LOD: big blobs, domains of reals, named keys
        let blob = Llsd::Binary(vec![7u8; 1 << 20]);
        let face = llsd_map! {
            "Position" => blob.clone(),
            "TriangleList" => blob,
            "PositionDomain" => llsd_map!{"Min" => Llsd::Array(vec![Llsd::Real(-0.5); 3]), "Max" => Llsd::Array(vec![Llsd::Real(0.5); 3])},
        };
        let b = to_binary(&Llsd::Array(vec![face; 4]));
        assert_eq!(from_binary(&b).unwrap().0.len(), 4);
        // many small maps with named keys and integers
        let rows = (0..20_000)
            .map(|i| llsd_map! {"offset" => i, "size" => i * 2, "name" => format!("joint{i}")})
            .collect::<Vec<_>>();
        assert_eq!(from_binary(&to_binary(&Llsd::Array(rows))).unwrap().0.len(), 20_000);
    }

    /// A document shaped like the viewer's inventory cache
    /// (`Inventory::save_cache`): `{version, folders, items}`, `count` items
    /// of 23 fields with short keys, built from one encoded item.
    fn inventory_cache(count: usize) -> Vec<u8> {
        let u = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        let item = to_binary(&llsd_map! {
            "id" => u, "parent" => u, "name" => "Wooden chair (copy)", "desc" => "",
            "asset_type" => 6, "inv_type" => 6, "asset_id" => u, "flags" => 0,
            "favorite" => false, "thumbnail" => Uuid::nil(), "base_mask" => -1, "owner_mask" => -1,
            "last_owner" => u, "group_id" => Uuid::nil(), "group_owned" => false,
            "sale_type" => 0, "sale_price" => 10, "creator" => u, "created_at" => 1_700_000_000,
            "owner" => u, "group_mask" => 0, "everyone_mask" => 0, "next_owner_mask" => 581632,
        });
        let head = to_binary(&llsd_map! {"version" => 3, "folders" => Llsd::Array(vec![])});
        // splice an "items" entry into the map: one more key, then the array
        let mut b = vec![b'{'];
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&head[5..head.len() - 1]);
        b.push(b'k');
        b.extend_from_slice(&5u32.to_be_bytes());
        b.extend_from_slice(b"items[");
        b.extend_from_slice(&(count as u32).to_be_bytes());
        for _ in 0..count {
            b.extend_from_slice(&item);
        }
        b.extend_from_slice(b"]}");
        b
    }

    #[test]
    fn inventory_cache_shape_stays_far_under_the_per_byte_rule() {
        let doc = inventory_cache(2000);
        let (v, used, left) = parse(&doc, Budget::for_local_input(doc.len())).unwrap();
        assert_eq!(used, doc.len());
        assert_eq!(v["items"].len(), 2000);
        assert_eq!(v["version"].as_i32(), 3);
        let spent = Budget::for_local_input(doc.len()).left - left.left;
        let ratio = spent as f64 / doc.len() as f64;
        println!(
            "inventory cache: {} bytes per item, {ratio:.2} bytes of tree per byte",
            doc.len() / 2000
        );
        assert!(ratio < 8.0, "{ratio}");
    }

    #[test]
    fn the_viewers_own_files_have_no_fixed_cap() {
        // 100,000 items: about 300 MB of estimated tree, over the cap
        let doc = inventory_cache(100_000);
        assert!(matches!(from_binary(&doc), Err(LlsdError::TooLarge)));
        let (v, used) = from_binary_local(&doc).unwrap();
        assert_eq!(used, doc.len());
        assert_eq!(v["items"].len(), 100_000);
        // the per-byte rule still holds there
        assert!(matches!(from_binary_local(&array_of(1 << 20, b"!")), Err(LlsdError::TooLarge)));
    }

    #[test]
    fn truncated_is_error() {
        let b = to_binary(&llsd_map! {"a" => "hello"});
        for n in 0..b.len() {
            assert!(from_binary(&b[..n]).is_err());
        }
    }
}
