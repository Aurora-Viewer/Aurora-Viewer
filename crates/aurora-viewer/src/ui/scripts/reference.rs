//! LLScriptEdCore::processKeywords / setHelpPage (Firestorm, LGPL 2.1).
//! Signatures and help come from the LSL keywords in use (the bundled
//! keywords_lsl_default.xml or the region's), or SLua's for a Lua script
//! (`crate::lsl_keywords`).

pub(super) use crate::lsl_keywords::Entry;

pub(super) fn all(lua: bool) -> &'static [Entry] {
    super::syntax::catalog(lua).entries()
}

pub(super) fn get(name: &str, lua: bool) -> Option<&'static Entry> {
    super::syntax::catalog(lua).entry(name)
}

/// build_script_help_url (indra/newview/llpreviewscript.cpp): `ll.` names
/// open the LSL wiki page of the same function (`ll.Say` → `llSay`), the
/// Luau libraries luau.org, any other SLua word the Lua portal.
pub(super) fn url(name: &str, lua: bool) -> String {
    // Keywords contain only identifier characters and dots; never interpolate source text.
    let known = get(name, lua).is_some();
    if !lua || name.starts_with("ll.") {
        return if known {
            format!("https://wiki.secondlife.com/wiki/{}", name.replacen("ll.", "ll", usize::from(lua)))
        } else if lua {
            "https://wiki.secondlife.com/wiki/Lua_Alpha".into()
        } else {
            "https://wiki.secondlife.com/wiki/LSL_Portal".into()
        };
    }
    const LIBRARIES: [&str; 9] = [
        "bit32.",
        "buffer.",
        "coroutine.",
        "debug.",
        "math.",
        "os.",
        "string.",
        "table.",
        "utf8.",
    ];
    if known && LIBRARIES.iter().any(|l| name.starts_with(l)) {
        "https://luau.org/library".into()
    } else {
        "https://wiki.secondlife.com/wiki/Lua_Alpha".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_is_sorted_complete_and_contains_signatures() {
        assert!(all(false).len() > 1100);
        assert!(all(false).windows(2).all(|w| w[0].name < w[1].name));
        let say = get("llSay", false).expect("native function");
        assert_eq!(say.signature, "void llSay(integer Channel, string Text)");
        assert!(!say.tooltip.is_empty());
        assert_eq!(get("NULL_KEY", false).expect("constant").data_type, "string");
        assert_eq!(url("user supplied/../text", false), "https://wiki.secondlife.com/wiki/LSL_Portal");
        assert_eq!(url("llSay", false), "https://wiki.secondlife.com/wiki/llSay");
    }

    #[test]
    fn slua_help_opens_the_lsl_page_luau_or_the_lua_portal() {
        assert!(get("ll.Say", true).is_some());
        assert!(get("llSay", true).is_none());
        assert_eq!(url("ll.Say", true), "https://wiki.secondlife.com/wiki/llSay");
        assert_eq!(url("string.format", true), "https://luau.org/library");
        assert_eq!(url("ll.NotAFunction", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
        assert_eq!(url("", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
        assert_eq!(url("local", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
    }
}
