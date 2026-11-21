//! The simulator's request to open a web window (llOpenFloater): the
//! "URLFloater" GenericMessage. Port of LLUrlFloaterDispatchHandler
//! (indra/newview/llurlfloaterdispatchhandler.cpp, originally LGPL 2.1).
//!
//! Only the `web_content` window exists for it; other names are logged and
//! dropped, as there. Its `floater_params` (`trusted_content`,
//! `show_page_title`, `allow_address_entry`) are placeholders the server
//! does not validate yet: Aurora reads none of them, so a page opened this
//! way never gets more trust than any other link.

/// GenericMessage method (MESSAGE_URL_FLOATER).
pub const METHOD: &str = "URLFloater";
/// The only window the request may open (FLOATER_WEB_CONTENT).
const WEB_CONTENT: &str = "web_content";

/// The URL to open from a URLFloater's first parameter (serialized LLSD),
/// or None when the message is malformed or names another window.
pub fn url(parameter: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(parameter);
    let text = text.trim_matches(char::from(0)).trim();
    // LLSDSerialize::deserialize: XML or notation (binary is never sent here)
    let message = if text.starts_with('<') {
        aurora_llsd::from_xml(text.as_bytes())
    } else {
        aurora_llsd::from_notation(text.as_bytes())
    };
    let message = match message {
        Ok(m) => m,
        Err(e) => {
            log::warn!("URLFloater: unreadable data ({e})");
            return None;
        }
    };
    let data = if message.get("action").to_string_value() == "OpenURL" {
        message.get("action_data")
    } else if message.has("floater_title") {
        &message
    } else {
        log::warn!("URLFloater: unexpected data format");
        return None;
    };
    let floater = data.get("floater_title").to_string_value();
    if floater != WEB_CONTENT {
        log::info!("URLFloater: the {floater:?} window is not supported");
        return None;
    }
    let mut url = data.get("floater_url").to_string_value();
    if !url.contains("://") {
        // "try unescaping"
        url = unescape(&url);
    }
    (!url.is_empty()).then_some(url)
}

/// LLURI::unescape: `%XX` decoded, everything else kept.
fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_url_action_gives_its_url() {
        let m = br#"{'action':'OpenURL','action_data':{'floater_title':'web_content','floater_url':'https://example.com/page','floater_params':{'trusted_content':1}}}"#;
        assert_eq!(url(m).as_deref(), Some("https://example.com/page"));
    }

    #[test]
    fn the_flat_form_and_xml_are_read_too() {
        let m = br#"{'floater_title':'web_content','floater_url':'https://example.com/a'}"#;
        assert_eq!(url(m).as_deref(), Some("https://example.com/a"));
        let xml = b"<?xml version=\"1.0\" ?><llsd><map><key>floater_title</key><string>web_content</string><key>floater_url</key><string>https://example.com/x</string></map></llsd>";
        assert_eq!(url(xml).as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn an_escaped_url_is_unescaped_and_others_are_kept() {
        let m = br#"{'floater_title':'web_content','floater_url':'https%3A%2F%2Fexample.com%2Fb%3Fq%3D1'}"#;
        assert_eq!(url(m).as_deref(), Some("https://example.com/b?q=1"));
        let m = br#"{'floater_title':'web_content','floater_url':'https://example.com/c%20d'}"#;
        assert_eq!(url(m).as_deref(), Some("https://example.com/c%20d"), "already a URL: left as it is");
    }

    #[test]
    fn another_window_an_empty_url_or_garbage_open_nothing() {
        assert_eq!(url(br#"{'floater_title':'preferences','floater_url':'https://example.com'}"#), None);
        assert_eq!(url(br#"{'floater_title':'web_content','floater_url':''}"#), None);
        assert_eq!(url(br#"{'something':'else'}"#), None);
        assert_eq!(url(b"not llsd {"), None);
    }
}
