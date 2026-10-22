//! User choices from the Legacy Viewer's RLVa settings. Actual
//! settings.xml defaults take precedence over fallback values in helpers.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub im_queries: bool,
    pub im_queries_auto_share: bool,
    pub allow_shared_offers: bool,
    pub allow_environment_locks: bool,
    pub temporary_controllers: bool,
    /// Native selector: 0 none, 1 general, 2 moderate, 3 adult.
    pub experience_maturity: u8,
    pub blocked_experiences: String,
    pub allow_ooc: bool,
    pub show_ellipsis: bool,
    pub split_redirect_chat: bool,
    pub show_redirect_typing: bool,
    pub replace_unlocked: bool,
    pub experimental_commands: bool,
    pub debug_commands: bool,
    pub debug_hide_unset_duplicate: bool,
    pub compatibility_list: String,
    pub legacy_naming: bool,
    pub shared_wear: bool,
    pub auto_rename: bool,
    pub hide_locked_attachments: bool,
    pub hide_locked_layers: bool,
    pub wear_add_prefix: String,
    pub wear_replace_prefix: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            im_queries: true,
            im_queries_auto_share: false,
            allow_shared_offers: true,
            allow_environment_locks: true,
            temporary_controllers: true,
            experience_maturity: 2,
            blocked_experiences: "bfe25fb4-222c-11e5-85a2-fa4c4ccaa202".into(),
            allow_ooc: true,
            show_ellipsis: true,
            split_redirect_chat: false,
            show_redirect_typing: false,
            replace_unlocked: true,
            experimental_commands: true,
            debug_commands: false,
            debug_hide_unset_duplicate: false,
            compatibility_list: String::new(),
            legacy_naming: true,
            shared_wear: true,
            auto_rename: true,
            hide_locked_attachments: false,
            hide_locked_layers: false,
            wear_add_prefix: "+".into(),
            wear_replace_prefix: String::new(),
        }
    }
}

impl Preferences {
    /// RlvBehaviourDictionary::addEntry's experimental feature gate.
    pub(super) fn command_available(&self, name: &str) -> bool {
        self.experimental_commands || !EXPERIMENTAL.contains(&name)
    }
    pub(super) fn replace(&self, name: &str, replace: bool) -> bool {
        let prefix = if replace {
            &self.wear_add_prefix
        } else {
            &self.wear_replace_prefix
        };
        if !prefix.is_empty() && !(prefix.len() > 1 && prefix.starts_with('.')) && name.starts_with(prefix) {
            !replace
        } else {
            replace
        }
    }
}

const EXPERIMENTAL: &[&str] = &[
    "allowidle",
    "viewtransparent",
    "viewwireframe",
    "sendchannel_except",
    "sendgesture",
    "shownearby",
    "showself",
    "showselfhead",
    "tplocal",
    "setcam_avdistmin",
    "setcam_avdistmax",
    "setcam_origindistmin",
    "setcam_origindistmax",
    "setcam_focus",
    "setcam_fov",
    "setcam_mode",
    "getaddattachnames",
    "getaddoutfitnames",
    "getattachnames",
    "getcam_avdist",
    "getcam_avdistmin",
    "getcam_avdistmax",
    "getcam_fov",
    "getcam_fovmin",
    "getcam_fovmax",
    "getcam_textures",
    "getoutfitnames",
    "getremattachnames",
    "getremoutfitnames",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_inventory_preferences_keep_reference_defaults_for_old_and_partial_settings() {
        let settings: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.rlva_enabled);
        assert!(settings.rlva.allow_ooc && settings.rlva.show_ellipsis);
        assert!(!settings.rlva.split_redirect_chat && settings.rlva.compatibility_list.is_empty());
        assert!(!settings.rlva.show_redirect_typing && settings.rlva.replace_unlocked && settings.rlva.experimental_commands);
        assert!(!settings.rlva.debug_commands && !settings.rlva.debug_hide_unset_duplicate);
        assert_eq!(settings.rlva.blocked_experiences, "bfe25fb4-222c-11e5-85a2-fa4c4ccaa202");
        assert_eq!(settings.rlva, Preferences::default());
        let preferences: Preferences = serde_json::from_str(r#"{"shared_wear":false}"#).unwrap();
        assert!(!preferences.shared_wear && preferences.legacy_naming);
        assert_eq!(preferences.wear_add_prefix, "+");
        assert!(!preferences.hide_locked_layers && !preferences.hide_locked_attachments);
        assert_eq!(
            serde_json::from_str::<Preferences>(&serde_json::to_string(&preferences).unwrap()).unwrap(),
            preferences
        );
        assert!(!preferences.replace("+Outfit", true));
        let prefs = Preferences {
            wear_replace_prefix: "!".into(),
            wear_add_prefix: ".hidden".into(),
            ..preferences
        };
        assert!(prefs.replace("!Outfit", false));
        assert!(prefs.replace(".hidden", true), "hidden prefixes never alter wear mode");
        let prefs = Preferences {
            wear_add_prefix: ".".into(),
            ..prefs
        };
        assert!(!prefs.replace(".explicit", true), "the reference accepts a single-dot prefix");
    }
}
