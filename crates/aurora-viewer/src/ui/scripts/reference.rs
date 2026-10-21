//! LLScriptEdCore::processKeywords / setHelpPage (Firestorm, LGPL 2.1).
//! Signatures and help come from the LSL keywords in use: the bundled
//! keywords_lsl_default.xml or the region's (`crate::lsl_keywords`).

pub(super) use crate::lsl_keywords::Entry;

pub(super) fn all() -> &'static [Entry] {
    crate::lsl_keywords::current().entries()
}

pub(super) fn get(name: &str) -> Option<&'static Entry> {
    crate::lsl_keywords::current().entry(name)
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
