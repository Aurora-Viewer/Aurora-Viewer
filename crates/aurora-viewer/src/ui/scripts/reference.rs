//! LLScriptEdCore::processKeywords / setHelpPage (Firestorm, LGPL 2.1).
//! Bundled signatures and help come from keywords_lsl_default.xml.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub(super) struct Entry {
    pub name: String,
    pub group: String,
    #[serde(rename = "type")]
    pub data_type: String,
    pub signature: String,
    pub tooltip: String,
}

pub(super) fn all() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| serde_json::from_str(include_str!("../../../assets/lsl/reference.json")).expect("valid bundled LSL reference"))
}

pub(super) fn get(name: &str) -> Option<&'static Entry> {
    all()
        .binary_search_by(|entry| entry.name.as_str().cmp(name))
        .ok()
        .map(|i| &all()[i])
}

pub(super) fn url(name: &str) -> String {
    // Keywords contain only identifier characters; never interpolate source text.
    if get(name).is_some() {
        format!("https://wiki.secondlife.com/wiki/{name}")
    } else {
        "https://wiki.secondlife.com/wiki/LSL_Portal".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_is_sorted_complete_and_contains_signatures() {
        assert!(all().len() > 1100);
        assert!(all().windows(2).all(|w| w[0].name < w[1].name));
        let say = get("llSay").expect("native function");
        assert_eq!(say.signature, "void llSay(integer Channel, string Text)");
        assert!(!say.tooltip.is_empty());
        assert_eq!(get("NULL_KEY").expect("constant").data_type, "string");
        assert_eq!(url("user supplied/../text"), "https://wiki.secondlife.com/wiki/LSL_Portal");
    }
}
