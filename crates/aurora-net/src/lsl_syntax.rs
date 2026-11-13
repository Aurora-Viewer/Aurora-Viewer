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

/// The cache file of a syntax's SLua keywords.
pub fn lua_cache_file(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(format!("keywords_lua_{id}.llsd.xml"))
}

/// A syntax's keyword documents: LSL's, and SLua's when the region gives
/// them. None leaves the bundled ones (a region may omit a language).
#[derive(Debug, Default)]
pub struct Definitions {
    pub lsl: Option<Llsd>,
    pub lua: Option<Llsd>,
}

async fn cached(file: &Path) -> Option<Llsd> {
    let bytes = tokio::fs::read(file).await.ok()?;
    match aurora_llsd::from_xml(&bytes) {
        Ok(doc) if is_supported(&doc) => Some(doc),
        _ => {
            log::warn!("lsl syntax: the cached {} cannot be read, fetching it again", file.display());
            None
        }
    }
}

async fn write_cache(dir: &Path, file: &Path, doc: &Llsd) {
    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        log::warn!("lsl syntax: no cache folder ({e})");
    } else if let Err(e) = tokio::fs::write(file, aurora_llsd::to_xml(doc)).await {
        log::warn!("lsl syntax: not cached ({e})");
    }
}

/// The syntax `id` through ScriptDefinitions: from the cache, else from the
/// capability, whose `files` map holds `lsl_keywords.xml` and
/// `lua_keywords.xml` (LLSyntaxDefCache::fetchKeywordsDefsCoro). A file of
/// an unknown version is left out like an omitted one.
pub async fn load_definitions(http: &reqwest::Client, cap_url: &str, dir: &Path, id: Uuid) -> Result<Definitions, String> {
    let (lsl_file, lua_file) = (cache_file(dir, id), lua_cache_file(dir, id));
    let cache = Definitions {
        lsl: cached(&lsl_file).await,
        lua: cached(&lua_file).await,
    };
    // both cached; an LSL file alone may come from LSLSyntax, which has no
    // SLua: ask once more (a region without SLua keywords costs one request
    // per session, lsl_syntax_asked)
    if cache.lsl.is_some() && cache.lua.is_some() {
        return Ok(cache);
    }
    let reply = match crate::caps::get_llsd(http, cap_url).await {
        Ok(reply) => reply,
        Err(_) if cache.lsl.is_some() => return Ok(cache),
        Err(e) => return Err(e.to_string()),
    };
    let files = &reply["files"];
    if files.as_map().is_none() {
        return if cache.lsl.is_some() {
            Ok(cache)
        } else {
            Err("no files in the ScriptDefinitions reply".into())
        };
    }
    let mut defs = Definitions::default();
    for (name, file, slot) in [
        ("lsl_keywords.xml", &lsl_file, &mut defs.lsl),
        ("lua_keywords.xml", &lua_file, &mut defs.lua),
    ] {
        let doc = &files[name];
        if is_supported(doc) {
            write_cache(dir, file, doc).await;
            *slot = Some(doc.clone());
        } else if !doc.is_undef() {
            log::warn!(
                "lsl syntax: {name} of unsupported version {}",
                doc["llsd-lsl-syntax-version"].to_string_value()
            );
        }
    }
    // a language the reply left out keeps what the cache had
    defs.lsl = defs.lsl.or(cache.lsl);
    defs.lua = defs.lua.or(cache.lua);
    Ok(defs)
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
            // SLua names carry their library: `ll.Say`, `bit32.band`
            // (LLKeywords::findSegments reads dots in Lua words)
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
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
    write_cache(dir, &file, &doc).await;
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

    #[tokio::test]
    async fn script_definitions_give_both_languages_and_are_cached() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("test listener");
        let url = format!("http://{}", listener.local_addr().expect("test address"));
        let mut lua = doc();
        if let Llsd::Map(lua) = &mut lua {
            let functions = aurora_llsd::from_xml(b"<llsd><map><key>ll.Say</key><map/></map></llsd>").expect("map");
            lua.insert("functions".into(), functions);
        }
        let mut files = aurora_llsd::Map::new();
        files.insert("lsl_keywords.xml".into(), doc());
        files.insert("lua_keywords.xml".into(), lua);
        let mut reply = aurora_llsd::Map::new();
        reply.insert("files".into(), Llsd::Map(files));
        let body = aurora_llsd::to_xml(&Llsd::Map(reply));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("definitions request");
            let mut data = Vec::new();
            while !data.windows(4).any(|b| b == b"\r\n\r\n") {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.expect("request bytes");
                assert!(n > 0, "request truncated");
                data.extend_from_slice(&buf[..n]);
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.expect("reply header");
            socket.write_all(&body).await.expect("reply body");
            crate::test_http::finish(socket);
        });
        let dir = std::env::temp_dir().join(format!("aurora-defs-{}", Uuid::new_v4()));
        let id = Uuid::from_u128(0xdef);
        let http = reqwest::Client::new();
        let defs = load_definitions(&http, &url, &dir, id).await.expect("definitions");
        server.await.expect("server");
        assert!(keywords(defs.lsl.as_ref().expect("LSL")).is_some_and(|k| k.iter().any(|k| k.name == "llSay")));
        let lua = keywords(defs.lua.as_ref().expect("SLua")).expect("SLua keywords");
        assert!(lua.iter().any(|k| k.name == "ll.Say" && k.group == Group::Function));
        // the second time comes from the cache: no server any more
        let cached = load_definitions(&http, "http://127.0.0.1:9/none", &dir, id).await.expect("cached");
        assert!(cached.lsl.is_some() && cached.lua.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_lsl_file_cached_alone_is_kept_when_the_definitions_cannot_be_fetched() {
        // LSLSyntax caches LSL alone: ScriptDefinitions is asked for SLua,
        // and when it fails the cached LSL still serves
        let dir = std::env::temp_dir().join(format!("aurora-defs-{}", Uuid::new_v4()));
        let id = Uuid::from_u128(0xdee);
        std::fs::create_dir_all(&dir).expect("test folder");
        std::fs::write(cache_file(&dir, id), aurora_llsd::to_xml(&doc())).expect("cached LSL");
        let http = reqwest::Client::new();
        let defs = load_definitions(&http, "http://127.0.0.1:9/none", &dir, id)
            .await
            .expect("cached LSL");
        assert!(defs.lsl.is_some() && defs.lua.is_none());
        let _ = std::fs::remove_dir_all(&dir);
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
