//! Plugin messages (LLPluginMessage): an LLSD map `{class, name, params}`
//! serialized as LLSD XML, one message per `\0`-terminated string.
//!
//! Value encodings follow llpluginmessage.cpp: U32 values are hex strings
//! ("0x80e1"), pointers are strings, everything else native LLSD.

use aurora_llsd::{Llsd, Map};

#[derive(Debug, Clone, Default)]
pub struct PluginMessage {
    pub class: String,
    pub name: String,
    pub params: Map,
}

impl PluginMessage {
    pub fn new(class: &str, name: &str) -> PluginMessage {
        PluginMessage {
            class: class.to_string(),
            name: name.to_string(),
            params: Map::new(),
        }
    }

    pub fn with(mut self, key: &str, v: impl Into<Llsd>) -> PluginMessage {
        self.params.insert(key.to_string(), v.into());
        self
    }

    pub fn set(&mut self, key: &str, v: impl Into<Llsd>) {
        self.params.insert(key.to_string(), v.into());
    }

    pub fn has(&self, key: &str) -> bool {
        self.params.contains_key(key)
    }

    pub fn get(&self, key: &str) -> &Llsd {
        static UNDEF: Llsd = Llsd::Undef;
        self.params.get(key).unwrap_or(&UNDEF)
    }

    pub fn string(&self, key: &str) -> String {
        self.get(key).to_string_value()
    }

    pub fn s32(&self, key: &str) -> i32 {
        self.get(key).as_i32()
    }

    pub fn real(&self, key: &str) -> f64 {
        self.get(key).as_f64()
    }

    pub fn boolean(&self, key: &str) -> bool {
        self.get(key).as_bool()
    }

    /// getValueU32: hex string.
    pub fn u32_hex(&self, key: &str) -> u32 {
        let s = self.string(key);
        let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        u32::from_str_radix(t, 16).unwrap_or(0)
    }

    pub fn generate(&self) -> String {
        let mut m = Map::new();
        m.insert("class".into(), Llsd::String(self.class.clone()));
        m.insert("name".into(), Llsd::String(self.name.clone()));
        m.insert("params".into(), Llsd::Map(self.params.clone()));
        aurora_llsd::to_xml_string(&Llsd::Map(m))
    }

    pub fn parse(text: &[u8]) -> Option<PluginMessage> {
        let v = aurora_llsd::from_xml(text).ok()?;
        let params = v.get("params").as_map().cloned().unwrap_or_default();
        Some(PluginMessage {
            class: v.get("class").to_string_value(),
            name: v.get("name").to_string_value(),
            params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let m = PluginMessage::new("media", "size_change")
            .with("name", "LL_1_0")
            .with("width", 1024)
            .with("background_r", 1.0)
            .with("format", "0x80e1");
        let p = PluginMessage::parse(m.generate().as_bytes()).unwrap();
        assert_eq!(p.class, "media");
        assert_eq!(p.name, "size_change");
        assert_eq!(p.s32("width"), 1024);
        assert_eq!(p.u32_hex("format"), 0x80e1);
        assert_eq!(p.string("name"), "LL_1_0");
    }

    #[test]
    fn parses_pretty_xml() {
        let src = "<?xml version=\"1.0\" ?>\n<llsd>\n<map>\n  <key>class</key>\n    <string>internal</string>\n  <key>name</key>\n    <string>hello</string>\n  <key>params</key>\n    <map>\n    </map>\n  </map>\n</llsd>\n";
        let p = PluginMessage::parse(src.as_bytes()).unwrap();
        assert_eq!((p.class.as_str(), p.name.as_str()), ("internal", "hello"));
    }
}
