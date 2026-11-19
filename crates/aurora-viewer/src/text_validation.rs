//! Shared reasons for restricted fields and non-UI mutation guards.
pub use aurora_net::text_validation::Policy;

pub fn reason(policy: Policy) -> &'static str {
    match policy {
        Policy::Ascii => aurora_i18n::tr!("input-restricted-ascii"),
        Policy::PrintableNoPipe => aurora_i18n::tr!("input-restricted-no-pipe"),
        Policy::MultilineNoPipe => aurora_i18n::tr!("input-restricted-multiline"),
        Policy::AoName => aurora_i18n::tr!("input-restricted-ao"),
        Policy::Digits => aurora_i18n::tr!("input-restricted-digits"),
        Policy::Decimal => aurora_i18n::tr!("input-restricted-decimal"),
    }
}

pub fn require(policy: Policy, text: &str) -> Result<(), String> {
    if policy.accepts(text) { Ok(()) } else { Err(reason(policy).into()) }
}

/// Prevalidate new input while retaining characters already received from
/// the server. Existing unsupported characters may remain or be deleted;
/// adding, duplicating or reordering them is still refused. Linear, without
/// allocating a diff, including several ASCII edits around a Unicode name.
pub fn require_edit(policy: Policy, before: &str, after: &str) -> Result<(), String> {
    let mut existing = before.chars().filter(|&c| !policy.accepts_char(c));
    for c in after.chars().filter(|&c| !policy.accepts_char(c)) {
        if !existing.any(|old| old == c) {
            return Err(reason(policy).into());
        }
    }
    Ok(())
}

/// LLFloaterNameDesc::postBuild (indra/newview/llfloaternamedesc.cpp):
/// generated upload labels replace unsupported UTF-8 bytes with '?', remove
/// controls, then trim. The local path itself is never modified.
pub fn generated_name(name: &str, limit: usize) -> String {
    name.bytes()
        .map(|b| {
            if b == b'|' || (b as i8) < 32 && b != b'\n' {
                '?'
            } else {
                char::from(b)
            }
        })
        .filter(|c| (' '..='~').contains(c))
        .collect::<String>()
        .trim()
        .chars()
        .take(limit)
        .collect()
}

/// A translated default that becomes an inventory name ("New folder", "New
/// outfit", an unnamed object). Inventory names are plain ASCII
/// (`Policy::Ascii`), so a translation that is not falls back to English,
/// as the Legacy Viewer's untranslated "New Folder" and "New Outfit"
/// (indra/newview/llviewerfoldertype.cpp): `generated_name` would turn
/// Japanese into a row of '?'.
pub fn default_name(key: &'static str) -> &'static str {
    default_name_in(aurora_i18n::language(), key)
}

pub fn default_name_in(language: &str, key: &'static str) -> &'static str {
    let local = aurora_i18n::text_in(language, key);
    if local.is_ascii() { local } else { aurora_i18n::text_in("en", key) }
}

/// "<outfit> (new)" for Save as: the suffix in English when the language's
/// is not plain ASCII. The outfit's own name is kept as it is.
pub fn new_variant_name_in(language: &str, base: &str) -> String {
    let suffix = aurora_i18n::tr_in!(language, "status-new", value0 = String::new());
    let language = if suffix.is_ascii() { language } else { "en" };
    aurora_i18n::tr_in!(language, "status-new", value0 = base.to_string())
}

#[cfg(test)]
mod tests {
    use crate::world::{
        appearance,
        inventory::{Inventory, actions},
    };
    use aurora_llsd::llsd_map;
    use std::collections::HashMap;
    use uuid::Uuid;

    #[test]
    fn restricted_text_metadata_mutations_reject_new_unicode_but_preserve_unchanged_server_values() {
        let mut inv = Inventory::default();
        let id = Uuid::from_u128(123);
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id,
            parent: Uuid::from_u128(2),
            name: "Existing 桜".into(),
            desc: "Existing 龙".into(),
            ..Default::default()
        }]);
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "New 龙" }).is_err());
        assert!(actions::patch(&inv, id, llsd_map! { "desc" => "New 桜" }).is_err());
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "Renamed 桜 ASCII", "desc" => "Edited 龙 ASCII" }).is_ok());
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "Existing 桜桜" }).is_err());
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "Existing 桜", "desc" => "Existing 龙" }).is_ok());
        assert!(actions::patch(&inv, id, llsd_map! { "desc" => "First\nSecond" }).is_ok());
        assert_eq!(actions::clean_name("桜").expect("received name"), "桜");
        assert!(actions::clean_name("A|B").is_err());
        assert_eq!(actions::clean_name(" ASCII (test) ").expect("name"), "ASCII (test)");
    }

    #[test]
    fn restricted_text_generated_folder_labels_are_cleaned_without_rejecting_creation() {
        for (language, _) in aurora_i18n::available_languages() {
            let name = super::default_name_in(language, "common-new-folder");
            let change = actions::create_folder(Uuid::from_u128(1), name).expect("localized folder");
            assert!(
                matches!(&change.operations[..], [aurora_net::inventory::operations::Operation::Folder { name, .. }] if !name.is_empty() && !name.contains('?') && super::Policy::PrintableNoPipe.accepts(name))
            );
        }
        let object_name = crate::build::task_inventory::contents_folder_name("★ Gift box");
        assert!(!object_name.is_empty());
        assert!(super::Policy::PrintableNoPipe.accepts(&object_name));
        assert!(actions::create_folder(Uuid::from_u128(1), &object_name).is_ok());
    }

    #[test]
    fn restricted_text_outfit_mutations_reject_invalid_names_before_planning_requests() {
        let agent = Uuid::from_u128(1);
        let mut inv = Inventory {
            owner: agent,
            ..Default::default()
        };
        appearance::seed_demo(&mut inv, agent);
        let base = appearance::base(&inv).expect("saved outfit");
        assert!(appearance::plan(&inv, appearance::Action::Save(Some("ASCII outfit".into())), &HashMap::new()).is_ok());
        assert!(appearance::category_plan(&inv, base, aurora_net::outfits::categories::Update::Rename("ASCII rename".into())).is_ok());
        for name in ["桜", "龙", "Outfit 桜龙", "Café"] {
            assert!(appearance::plan(&inv, appearance::Action::Save(Some(name.into())), &HashMap::new()).is_err());
            assert!(appearance::category_plan(&inv, base, aurora_net::outfits::categories::Update::Rename(name.into())).is_err());
        }
        assert!(crate::ao::inventory::valid_name("Set 桜").is_err());
        assert!(crate::ao::inventory::valid_name("Set|Group").is_err());
        assert!(aurora_net::groups::name::checked("A桜龙").is_none());
    }
    #[test]
    fn default_names_stay_plain_ascii_in_every_language() {
        for (language, _) in aurora_i18n::available_languages() {
            for key in [
                "common-new-folder",
                "inventory-new-outfit",
                "ui-appearance-new-outfit",
                "common-object",
                "wearable-favorites-new-folder-default",
            ] {
                let name = super::default_name_in(language, key);
                assert!(
                    name.is_ascii() && !name.is_empty() && !name.contains('?'),
                    "{language} {key}: {name}"
                );
            }
            let favorites_default = super::default_name_in(language, "wearable-favorites-new-folder-default");
            assert_eq!(
                crate::world::wearable_favorites::folder_name(favorites_default).unwrap(),
                favorites_default
            );
            let variant = super::new_variant_name_in(language, "Party");
            assert!(variant.is_ascii() && !variant.contains('?'), "{language}: {variant}");
            assert!(variant.starts_with("Party") && variant != "Party", "{language}: {variant}");
        }
        assert_eq!(
            super::default_name_in("ja", "inventory-new-outfit"),
            aurora_i18n::text_in("en", "inventory-new-outfit")
        );
        assert_eq!(
            super::default_name_in("fr", "common-new-folder"),
            aurora_i18n::text_in("fr", "common-new-folder")
        );
        assert_eq!(
            super::new_variant_name_in("ja", "Party"),
            aurora_i18n::tr_in!("en", "status-new", value0 = "Party".to_string())
        );
        assert_eq!(
            super::new_variant_name_in("fr", "Party"),
            aurora_i18n::tr_in!("fr", "status-new", value0 = "Party".to_string())
        );
    }
}
