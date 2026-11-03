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
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "New 桜" }).is_err());
        assert!(actions::patch(&inv, id, llsd_map! { "desc" => "New 龙" }).is_err());
        assert!(actions::patch(&inv, id, llsd_map! { "name" => "Existing 桜", "desc" => "Existing 龙" }).is_ok());
        assert!(actions::patch(&inv, id, llsd_map! { "desc" => "First\nSecond" }).is_ok());
        assert!(actions::clean_name("桜").is_err());
        assert!(actions::clean_name("A|B").is_err());
        assert_eq!(actions::clean_name(" ASCII (test) ").expect("name"), "ASCII (test)");
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
}
