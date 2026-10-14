//! LLWearable::importStream / exportStream (Firestorm indra/llappearance,
//! originally LGPL 2.1). Keep permission, sale, texture and future sections
//! byte-for-byte when editing the visual parameters of an existing asset.

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Wearable {
    pub kind: u8,
    pub params: BTreeMap<i32, f32>,
    prefix: String,
    suffix: String,
}

impl Wearable {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > 64 * 1024 {
            return Err("wearable too large");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid wearable text")?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() < 5 || !lines[0].starts_with("LLWearable version ") {
            return Err("invalid wearable header");
        }
        let section = lines
            .iter()
            .enumerate()
            .skip(3)
            .find_map(|(index, line)| line.trim().starts_with("parameters ").then_some(index))
            .ok_or("missing wearable parameters")?;
        let kind = lines[3..section]
            .iter()
            .find_map(|l| l.trim().strip_prefix("type "))
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|k| *k < 17)
            .ok_or("invalid wearable type")?;
        let count = lines[section]
            .trim()
            .strip_prefix("parameters ")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n <= 2048)
            .ok_or("invalid parameter count")?;
        let end = section
            .checked_add(1 + count)
            .filter(|e| *e < lines.len())
            .ok_or("truncated wearable")?;
        let mut params = BTreeMap::new();
        for line in &lines[section + 1..end] {
            let mut words = line.split_whitespace();
            let id = words
                .next()
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|id| *id >= 0)
                .ok_or("invalid parameter id")?;
            let value = words
                .next()
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or("invalid parameter value")?;
            if words.next().is_some() || params.insert(id, value).is_some() {
                return Err("duplicate or malformed parameter");
            }
        }
        let textures = lines[end]
            .trim()
            .strip_prefix("textures ")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n <= 256 && end + 1 + n <= lines.len())
            .ok_or("invalid texture count")?;
        for line in &lines[end + 1..end + 1 + textures] {
            let mut words = line.split_whitespace();
            if words.next().and_then(|s| s.parse::<u32>().ok()).is_none()
                || words.next().and_then(|s| s.parse::<uuid::Uuid>().ok()).is_none()
                || words.next().is_some()
            {
                return Err("invalid wearable texture");
            }
        }
        // Offsets preserve the original line endings and every other section.
        let offset = |line: usize| text.split_inclusive('\n').take(line).map(str::len).sum::<usize>();
        Ok(Self {
            kind,
            params,
            prefix: text[..offset(section)].into(),
            suffix: text[offset(end)..].into(),
        })
    }

    pub fn encode(&self, name: &str) -> Result<Vec<u8>, &'static str> {
        if name.trim().is_empty()
            || name.contains(['\r', '\n', '\0'])
            || name.len() > 255
            || self.params.len() > 2048
            || self.params.values().any(|v| !v.is_finite())
        {
            return Err("invalid wearable contents");
        }
        let mut lines = self.prefix.splitn(3, '\n');
        let version = lines.next().ok_or("invalid wearable header")?;
        let _old_name = lines.next().ok_or("invalid wearable header")?;
        let metadata = lines.next().ok_or("invalid wearable metadata")?;
        let mut text = format!("{version}\n{name}\n{metadata}parameters {}\n", self.params.len());
        for (id, value) in &self.params {
            text.push_str(&format!("{id} {value}\n"));
        }
        text.push_str(&self.suffix);
        if text.len() > 64 * 1024 {
            return Err("wearable too large");
        }
        Ok(text.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ASSET: &str = "LLWearable version 22\nOriginal\nDescription\npermissions 0\n{\nowner_mask 00004000\n}\nsale_info 0\n{\nsale_type not\n}\ntype 0\nparameters 3\n33 0.5\n80 1\n9999 -2\ntextures 1\n0 00000000-0000-0000-0000-000000000001\nfuture section\n";
    #[test]
    fn edits_preserve_permissions_textures_unknown_parameters_and_future_sections() {
        let mut asset = Wearable::parse(ASSET.as_bytes()).expect("wearable");
        asset.params.insert(33, 1.75);
        let bytes = asset.encode("Edited").expect("encoded");
        let copy = Wearable::parse(&bytes).expect("round trip");
        assert_eq!(copy.params[&9999], -2.0);
        assert_eq!(copy.params[&33], 1.75);
        assert_eq!(copy.suffix, asset.suffix);
        assert!(String::from_utf8(bytes).expect("UTF-8").contains("owner_mask 00004000"));
        // The name and description are text, even when they resemble keys.
        for name in ["parameters 50", "type 4"] {
            let bytes = asset.encode(name).expect("name");
            let copy = Wearable::parse(&bytes).expect("named shape");
            assert_eq!(copy.kind, 0);
            assert_eq!(copy.params, asset.params);
        }
    }
    #[test]
    fn rejects_truncation_duplicates_nonfinite_values_and_header_injection() {
        for text in [
            ASSET.replace("33 0.5", "33 NaN"),
            ASSET.replace("80 1", "33 1"),
            ASSET.replace("parameters 3", "parameters 999999"),
        ] {
            assert!(Wearable::parse(text.as_bytes()).is_err());
        }
        let asset = Wearable::parse(ASSET.as_bytes()).expect("wearable");
        assert!(asset.encode("Shape\npermissions 0").is_err());
        assert!(Wearable::parse(b"LLWearable version 22\nMissing").is_err());
    }
}
