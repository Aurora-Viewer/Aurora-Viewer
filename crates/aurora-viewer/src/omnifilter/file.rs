//! The Omnifilter's file, `omnifilter.xml`, in the Legacy Viewer's LLSD
//! format (OmnifilterEngine::importFromLLSD, importRuleSetFromLLSD,
//! exportToLLSD and the older reading of loadNeedles, omnifilterengine.cpp,
//! originally LGPL 2.1), so that a Legacy file reads as it is:
//! - versions 3 and 4: a map of rule sets by name, each
//!   `{ order, rule_set: { <rule name>: { order, sender_name, … } } }`;
//! - versions 1 and 2 (before rule sets): the map of rules alone, which
//!   becomes the Default set (also when one of its rules is named "Default").
//!
//! Without an `order`, the map's order (by name) is kept, as there.

use super::{MatchType, Rule, RuleSet};
use aurora_llsd::Llsd;

/// The rule sets of a file, in their order.
pub fn parse(bytes: &[u8]) -> Result<Vec<RuleSet>, String> {
    let root = aurora_llsd::from_xml(bytes).map_err(|e| e.to_string())?;
    let Some(map) = root.as_map() else {
        return Err("not a map of rule sets".into());
    };
    // importFromLLSD: a "Default" set with an order, or the older format.
    // Stricter than the Legacy Viewer, which looks at the order alone: in a
    // version 2 file a rule named "Default" has an order too, so the set
    // must also hold its `rule_set` (a rule never does).
    let current = root.get(super::DEFAULT_SET);
    if !(current.is_map() && current.has("order") && current.has("rule_set")) {
        return Ok(vec![RuleSet {
            name: super::DEFAULT_SET.to_owned(),
            rules: rules_of(&root),
        }]);
    }
    let mut sets: Vec<(i64, usize, RuleSet)> = map
        .iter()
        .enumerate()
        .map(|(i, (name, v))| {
            let order = if v.has("order") {
                i64::from(v.get("order").as_i32())
            } else {
                i as i64
            };
            let set = RuleSet {
                name: name.clone(),
                rules: rules_of(v.get("rule_set")),
            };
            (order, i, set)
        })
        .collect();
    sets.sort_by_key(|(order, i, _)| (*order, *i));
    Ok(sets.into_iter().map(|(_, _, s)| s).collect())
}

/// importRuleSetFromLLSD: the rules of one set; a missing field keeps the
/// rule's default.
fn rules_of(v: &Llsd) -> Vec<Rule> {
    let Some(map) = v.as_map() else {
        return Vec::new();
    };
    let mut rules: Vec<(i64, usize, Rule)> = map
        .iter()
        .enumerate()
        .filter(|(_, (_, data))| data.is_map())
        .map(|(i, (name, data))| {
            let mut r = Rule {
                name: name.clone(),
                ..Rule::default()
            };
            let text = |key: &str, into: &mut String| {
                if data.has(key) {
                    *into = data.get(key).to_string_value();
                }
            };
            text("sender_name", &mut r.sender_name);
            text("content", &mut r.content);
            text("region_name", &mut r.region_name);
            text("chat_replace", &mut r.chat_replace);
            text("button_reply", &mut r.button_reply);
            text("textbox_reply", &mut r.textbox_reply);
            if data.has("sender_name_match_type") {
                r.sender_match = MatchType::from_i32(data.get("sender_name_match_type").as_i32());
            }
            if data.has("content_match_type") {
                r.content_match = MatchType::from_i32(data.get("content_match_type").as_i32());
            }
            for t in data.get("types").as_array() {
                r.add_source_number(t.as_i32());
            }
            r.enabled = data.get("enabled").as_bool();
            r.sender_case_insensitive = data.get("sender_name_case_insensitive").as_bool();
            r.content_case_insensitive = data.get("content_case_insensitive").as_bool();
            // version 4 (FIRE-36590)
            if data.has("reply_delay") {
                r.reply_delay = data.get("reply_delay").as_f32();
            }
            r.owner_id = data.get("owner_id").as_uuid();
            let order = if data.has("order") {
                i64::from(data.get("order").as_i32())
            } else {
                i as i64
            };
            (order, i, r)
        })
        .collect();
    rules.sort_by_key(|(order, i, _)| (*order, *i));
    rules.into_iter().map(|(_, _, r)| r).collect()
}

/// exportToLLSD: every set with its order, every rule with its own.
pub fn to_llsd(sets: &[RuleSet]) -> Llsd {
    let mut root = Llsd::new_map();
    for (order, set) in sets.iter().enumerate() {
        let mut rules = Llsd::new_map();
        for (i, r) in set.rules.iter().enumerate() {
            let mut n = Llsd::new_map();
            n.insert("order", i as i32);
            n.insert("sender_name", r.sender_name.clone());
            n.insert("content", r.content.clone());
            n.insert("region_name", r.region_name.clone());
            n.insert("chat_replace", r.chat_replace.clone());
            n.insert("button_reply", r.button_reply.clone());
            n.insert("textbox_reply", r.textbox_reply.clone());
            n.insert("sender_name_match_type", r.sender_match.as_i32());
            n.insert("content_match_type", r.content_match.as_i32());
            n.insert("owner_id", r.owner_id);
            let mut types = Llsd::new_array();
            for t in r.source_numbers() {
                types.push(t);
            }
            n.insert("types", types);
            n.insert("enabled", r.enabled);
            n.insert("sender_name_case_insensitive", r.sender_case_insensitive);
            n.insert("content_case_insensitive", r.content_case_insensitive);
            n.insert("reply_delay", f64::from(r.reply_delay));
            rules.insert(r.name.clone(), n);
        }
        let mut s = Llsd::new_map();
        s.insert("rule_set", rules);
        s.insert("order", order as i32);
        root.insert(set.name.clone(), s);
    }
    root
}

pub fn to_xml(sets: &[RuleSet]) -> String {
    aurora_llsd::to_xml_string(&to_llsd(sets))
}
