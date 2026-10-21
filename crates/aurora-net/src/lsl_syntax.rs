//! The LSL keywords of the simulator: SimulatorFeatures names the syntax in
//! use (`LSLSyntaxId`), the LSLSyntax capability serves it, and the viewer
//! keeps it in its cache under that id.
//!
//! Port of LLSyntaxIdLSL (indra/newview/llsyntaxid.cpp) and of the reading
//! of LLKeywords::processTokens (indra/llui/llkeywords.cpp), originally
//! LGPL 2.1. The document has the format of keywords_lsl_default.xml, the
//! catalog the viewer bundles: `controls`, `types`, `constants`, `events`
//! and `functions` maps, and `llsd-lsl-syntax-version` 2.

use aurora_llsd::Llsd;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// LLSD_SYNTAX_LSL_VERSION_EXPECTED: the format of the document, not its
/// content.
pub const VERSION: i32 = 2;

/// The section of the document a keyword comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Control,
    Type,
    Constant,
    Event,
    Function,
}

impl Group {
    /// The section's name, as the bundled reference writes it.
    pub fn name(self) -> &'static str {
        match self {
            Group::Control => "controls",
            Group::Type => "types",
            Group::Constant => "constants",
            Group::Event => "events",
            Group::Function => "functions",
        }
    }
}

/// One keyword with what the editor shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyword {
    pub name: String,
    pub group: Group,
    /// A constant's type; empty for the other groups.
    pub data_type: String,
    /// `integer llAbs(integer Value)`, `timer()`, `float PI = 3.14159265`.
    pub signature: String,
    pub tooltip: String,
}

/// The cache file of a syntax (LLSyntaxIdLSL::buildFullFileSpec).
pub fn cache_file(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(format!("keywords_lsl_{id}.llsd.xml"))
}

/// isSupportedVersion: the document's format is the one we read.
pub fn is_supported(doc: &Llsd) -> bool {
    doc["llsd-lsl-syntax-version"].as_i32() == VERSION
}

/// The keywords of a document, sorted by name; None when its version is not
/// [`VERSION`] or it holds no keyword.
pub fn keywords(doc: &Llsd) -> Option<Vec<Keyword>> {
    if !is_supported(doc) {
        return None;
    }
    let mut out = Vec::new();
    for group in [Group::Control, Group::Type, Group::Constant, Group::Event, Group::Function] {
        let Some(section) = doc[group.name()].as_map() else {
            continue;
        };
        for (name, entry) in section {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            out.push(keyword(name, group, entry));
        }
    }
    if out.is_empty() {
        return None;
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    Some(out)
}

/// The tooltips and values keep `\n` as two characters
/// (LLKeywords::processTokens turns them into line breaks).
fn text(v: &Llsd) -> String {
    v.to_string_value().replace("\\n", "\n")
}

fn keyword(name: &str, group: Group, entry: &Llsd) -> Keyword {
    let arguments = || {
        entry["arguments"]
            .as_array()
            .iter()
            .filter_map(Llsd::as_map)
            .flat_map(|arg| arg.iter())
            .map(|(arg, info)| format!("{} {arg}", info["type"].to_string_value()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let data_type = if group == Group::Constant {
        entry["type"].to_string_value()
    } else {
        String::new()
    };
    let signature = match group {
        Group::Control | Group::Type => name.to_owned(),
        Group::Constant => format!("{data_type} {name} = {}", text(&entry["value"])),
        Group::Event => format!("{name}({})", arguments()),
        Group::Function => {
            let ret = entry["return"].to_string_value();
            let ret = if ret.is_empty() { "void".to_owned() } else { ret };
            format!("{ret} {name}({})", arguments())
        }
    };
    Keyword {
        name: name.to_owned(),
        group,
        data_type,
        signature,
        tooltip: text(&entry["tooltip"]),
    }
}

/// The syntax `id`: from the cache, else from the capability (then cached).
/// LLSyntaxIdLSL::initialize / fetchKeywordsFileCoro / cacheFile.
pub async fn load(http: &reqwest::Client, cap_url: Option<&str>, dir: &Path, id: Uuid) -> Result<Llsd, String> {
    let file = cache_file(dir, id);
    if let Ok(bytes) = tokio::fs::read(&file).await {
        match aurora_llsd::from_xml(&bytes) {
            Ok(doc) if is_supported(&doc) => return Ok(doc),
            _ => log::warn!("lsl syntax: the cached {} cannot be read, fetching it again", file.display()),
        }
    }
    let url = cap_url.ok_or("no LSLSyntax capability")?;
    let doc = crate::caps::get_llsd(http, url).await.map_err(|e| e.to_string())?;
    if !is_supported(&doc) {
        return Err(format!("unsupported version {}", doc["llsd-lsl-syntax-version"].to_string_value()));
    }
    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        log::warn!("lsl syntax: no cache folder ({e})");
    } else if let Err(e) = tokio::fs::write(&file, aurora_llsd::to_xml(&doc)).await {
        log::warn!("lsl syntax: not cached ({e})");
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?><llsd><map>
        <key>controls</key><map><key>if</key><map><key>tooltip</key><string>if block\nif (x) {}</string></map></map>
        <key>types</key><map><key>integer</key><map><key>tooltip</key><string>32 bit</string></map></map>
        <key>constants</key><map>
          <key>ACTIVE</key><map><key>type</key><string>integer</string><key>value</key><string>0x2</string><key>tooltip</key><string>Moving.</string></map>
          <key>PI</key><map><key>type</key><string>float</string><key>value</key><real>3.14159265</real><key>tooltip</key><string/></map>
          <key>NULL_KEY</key><map><key>type</key><string>string</string><key>value</key><uuid>00000000-0000-0000-0000-000000000000</uuid></map>
          <key>EOF</key><map><key>type</key><string>string</string><key>value</key><string>\\n\\n\\n</string></map>
        </map>
        <key>events</key><map>
          <key>timer</key><map><key>arguments</key><undef/><key>tooltip</key><string>Tick.</string></map>
          <key>touch_start</key><map><key>arguments</key><array><map><key>NumberOfTouches</key><map><key>type</key><string>integer</string></map></map></array></map>
        </map>
        <key>functions</key><map>
          <key>llSay</key><map><key>return</key><string>void</string><key>arguments</key><array>
            <map><key>Channel</key><map><key>type</key><string>integer</string></map></map>
            <map><key>Text</key><map><key>type</key><string>string</string></map></map>
          </array><key>tooltip</key><string>Says Text.</string></map>
          <key>llNewThing</key><map><key>arguments</key><undef/></map>
          <key>bad name</key><map/>
        </map>
        <key>llsd-lsl-syntax-version</key><integer>2</integer>
    </map></llsd>"#;

    fn doc() -> Llsd {
        aurora_llsd::from_xml(DOC.as_bytes()).expect("test document")
    }

    fn get<'a>(words: &'a [Keyword], name: &str) -> &'a Keyword {
        words.iter().find(|k| k.name == name).expect(name)
    }

    #[test]
    fn the_signatures_read_like_the_bundled_reference() {
        let words = keywords(&doc()).expect("keywords");
        assert!(words.windows(2).all(|w| w[0].name < w[1].name));
        assert_eq!(get(&words, "llSay").signature, "void llSay(integer Channel, string Text)");
        assert_eq!(get(&words, "llNewThing").signature, "void llNewThing()");
        assert_eq!(get(&words, "timer").signature, "timer()");
        assert_eq!(get(&words, "touch_start").signature, "touch_start(integer NumberOfTouches)");
        assert_eq!(get(&words, "ACTIVE").signature, "integer ACTIVE = 0x2");
        assert_eq!(get(&words, "PI").signature, "float PI = 3.14159265");
        assert_eq!(get(&words, "PI").data_type, "float");
        assert_eq!(
            get(&words, "NULL_KEY").signature,
            "string NULL_KEY = 00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(get(&words, "EOF").signature, "string EOF = \\\n\\\n\\\n");
        assert_eq!(get(&words, "if").tooltip, "if block\nif (x) {}");
        assert_eq!(
            (get(&words, "integer").group, get(&words, "integer").signature.as_str()),
            (Group::Type, "integer")
        );
        assert!(words.iter().all(|k| k.name != "bad name"));
    }

    #[test]
    fn another_version_is_not_read() {
        let mut d = doc();
        if let Llsd::Map(m) = &mut d {
            m.insert("llsd-lsl-syntax-version".into(), Llsd::Integer(3));
        }
        assert!(!is_supported(&d));
        assert_eq!(keywords(&d), None);
        assert_eq!(keywords(&Llsd::Undef), None);
    }

    #[test]
    fn the_cache_file_is_named_after_the_syntax() {
        let id = Uuid::from_u128(0xabc);
        assert_eq!(
            cache_file(Path::new("cache"), id),
            Path::new("cache").join("keywords_lsl_00000000-0000-0000-0000-000000000abc.llsd.xml")
        );
    }
}
