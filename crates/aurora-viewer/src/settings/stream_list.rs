//! The saved music streams (`AudioSettings::saved_streams`) in the Legacy
//! Viewer's stream list file: its FSStreamList setting, a map with the URLs
//! under "audio" and "version" 1, written as pretty LLSD XML. Port of
//! FSStreamListExportXML / FSStreamListImportXML
//! (indra/newview/llviewermenu.cpp, originally LGPL 2.1); the file opens in
//! both viewers.
use aurora_llsd::Llsd;

/// The file name the save dialog offers (`stream_list.xml`).
pub const FILE_NAME: &str = "stream_list.xml";

/// The list as the Legacy Viewer writes its FSStreamList.
pub fn to_xml(streams: &[String]) -> String {
    let mut list = Llsd::new_map();
    list.insert("audio", Llsd::Array(streams.iter().map(|s| Llsd::from(s.as_str())).collect()));
    list.insert("version", 1);
    aurora_llsd::to_pretty_xml_string(&list)
}

/// The URLs of a stream list file, in their order. The Legacy Viewer takes
/// any LLSD and puts it in place of its list; here a file that is not a map
/// (another LLSD file, not LLSD at all) is refused, so that a wrong pick
/// does not empty the list. A map without "audio" is the empty list the
/// Legacy Viewer writes before a stream was ever saved.
pub fn from_xml(bytes: &[u8]) -> Option<Vec<String>> {
    let list = aurora_llsd::from_xml(bytes).ok()?;
    if !list.is_map() {
        return None;
    }
    let mut streams: Vec<String> = Vec::new();
    for url in list.get("audio").as_array() {
        let url = url.to_string_value();
        let url = url.trim();
        // onBtnStreamAdd never saves an empty URL nor the same one twice
        if !url.is_empty() && !streams.iter().any(|s| s == url) {
            streams.push(url.to_owned());
        }
    }
    Some(streams)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_legacy_format() {
        let xml = to_xml(&["http://a.example/stream".into(), "https://b.example/?x=1&y=2".into()]);
        assert_eq!(
            xml,
            "<?xml version=\"1.0\" ?>\n<llsd>\n    <map>\n    <key>audio</key>\n        <array>\n            \
             <string>http://a.example/stream</string>\n            <string>https://b.example/?x=1&amp;y=2</string>\n        \
             </array>\n    <key>version</key>\n        <integer>1</integer>\n    </map>\n</llsd>\n"
        );
    }

    #[test]
    fn reads_a_legacy_file() {
        // as LLSDSerialize::toPrettyXML writes the FSStreamList setting
        let file = b"<?xml version=\"1.0\" ?>\r\n<llsd>\r\n    <map>\r\n    <key>audio</key>\r\n        <array>\r\n            \
            <string>http://one.example:8000/live</string>\r\n            <string>two.example/radio</string>\r\n        \
            </array>\r\n    <key>version</key>\r\n        <integer>1</integer>\r\n    </map>\r\n</llsd>\r\n";
        assert_eq!(
            from_xml(file),
            Some(vec!["http://one.example:8000/live".to_owned(), "two.example/radio".to_owned()])
        );
    }

    #[test]
    fn round_trip() {
        let streams = vec!["http://a.example/".to_owned(), "http://b.example/<odd>".to_owned()];
        assert_eq!(from_xml(to_xml(&streams).as_bytes()), Some(streams));
        assert_eq!(from_xml(to_xml(&[]).as_bytes()), Some(Vec::new()));
    }

    #[test]
    fn empty_legacy_list_and_wrong_files() {
        // the setting's default, never written to
        assert_eq!(
            from_xml(b"<?xml version=\"1.0\" ?>\n<llsd>\n    <map />\n</llsd>\n"),
            Some(Vec::new())
        );
        assert_eq!(from_xml(b"<llsd><array><string>x</string></array></llsd>"), None);
        assert_eq!(from_xml(b"not xml at all"), None);
        assert_eq!(from_xml(b"{\"audio\": []}"), None);
    }

    #[test]
    fn skips_blank_and_repeated_urls() {
        let file = b"<llsd><map><key>audio</key><array><string> http://a.example/ </string><string />\
            <string>http://a.example/</string><string>http://b.example/</string></array></map></llsd>";
        assert_eq!(
            from_xml(file),
            Some(vec!["http://a.example/".to_owned(), "http://b.example/".to_owned()])
        );
    }
}
