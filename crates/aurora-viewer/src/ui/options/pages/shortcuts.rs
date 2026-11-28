//! Preferences › Shortcuts: every rebindable action, two bindings each,
//! one tab per section of `keybinds::sections` (the chat fields' keys under
//! Communication, the inventory lists' keys in a tab of their own, the
//! small sections together under "Other"). The search
//! of the window finds an action by its name, its section or one of its
//! keys.

use super::super::binding_cell;
use super::super::cx::{Custom, Cx};
use crate::keybinds::Action;
use crate::settings::Settings;
use std::ops::Range;

/// The first section of each tab with a name of its own: Movement, Camera,
/// Communication (with the chat fields' keys, the section after it),
/// Windows and Inventory. The sections after the last share "Other".
const TAB_STARTS: [usize; 5] = [0, 1, 2, 4, 5];

/// The sections each tab shows, in order.
fn tab_sections(sections: usize) -> Vec<Range<usize>> {
    let last = TAB_STARTS[TAB_STARTS.len() - 1];
    let bounds: Vec<usize> = TAB_STARTS.iter().chain([&(last + 1)]).map(|&i| i.min(sections)).collect();
    let mut tabs: Vec<Range<usize>> = bounds.windows(2).map(|w| w[0]..w[1]).collect();
    if (last + 1) < sections {
        tabs.push(last + 1..sections);
    }
    tabs
}

pub fn tabs() -> Vec<&'static str> {
    let sections = crate::keybinds::sections();
    tab_sections(sections.len())
        .iter()
        .enumerate()
        .map(|(i, range)| {
            if i < TAB_STARTS.len() {
                sections[range.start].0
            } else {
                aurora_i18n::tr!("ui-options-other-shortcuts")
            }
        })
        .collect()
}

pub fn page(cx: &mut Cx, tab: usize) {
    let p = cx.p;
    let sections = crate::keybinds::sections();
    let shown = tab_sections(sections.len())
        .get(tab)
        .map_or(&sections[..0], |range| &sections[range.clone()]);
    for (section, actions) in shown {
        cx.group(section, |cx| {
            if actions.iter().any(|(action, _)| *action == Action::HudDrag) {
                cx.note(aurora_i18n::tr!("hud-local-scale-help"));
            }
            for (a, label) in actions {
                let a = *a;
                let t = Custom {
                    path: "keybinds",
                    modified: move |s: &Settings, d: &Settings| s.keybinds.get(a) != d.keybinds.get(a),
                    reset: move |s: &mut Settings, d: &Settings| {
                        for (slot, binding) in d.keybinds.get(a).into_iter().enumerate() {
                            s.keybinds.set(a, slot, binding);
                        }
                    },
                };
                // the keys as the search reads them (their labels are costly:
                // only while searching)
                if cx.searching() {
                    let keys: Vec<String> = cx.s.keybinds.get(a).iter().flatten().map(|b| b.label()).collect();
                    cx.keywords(&keys.join(" "));
                }
                // one link per action: the row shares `keybinds` with the others
                cx.link_as(format_args!("keybinds.{a:?}"))
                    .wide()
                    .row_with(label, "", &t, |ui, s, st, _| {
                        for slot in 0..2 {
                            binding_cell(ui, &p, &s.keybinds, st, a, slot);
                        }
                        false
                    });
            }
        });
    }
    if let Some((a, slot)) = cx.st.clear.take() {
        cx.s.keybinds.set(a, slot, None);
        cx.changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chat_keys_sit_in_the_communication_tab() {
        let sections = crate::keybinds::sections();
        let tabs = tab_sections(sections.len());
        assert_eq!(tabs.len(), super::tabs().len());
        let communication = tabs
            .iter()
            .find(|r| sections[r.start].0 == aurora_i18n::tr!("common-communication"))
            .expect("a Communication tab");
        let actions: Vec<Action> = sections[communication.clone()]
            .iter()
            .flat_map(|(_, list)| list.iter().map(|(a, _)| *a))
            .collect();
        assert!(actions.contains(&Action::Chat) && actions.contains(&Action::PushToTalk));
        for a in actions.iter().filter(|a| crate::keybinds::is_chat(**a)) {
            assert!(actions.contains(a));
        }
        let chat_keys = sections
            .iter()
            .flat_map(|(_, list)| list.iter().map(|(a, _)| *a))
            .filter(|a| crate::keybinds::is_chat(*a))
            .count();
        assert_eq!(actions.iter().filter(|a| crate::keybinds::is_chat(**a)).count(), chat_keys);
        // every section is in exactly one tab, in order
        let flat: Vec<usize> = tabs.iter().flat_map(|r| r.clone()).collect();
        assert_eq!(flat, (0..sections.len()).collect::<Vec<_>>());
        // the other tabs keep one section each, then "Other"
        assert_eq!(sections[tabs[0].start].0, aurora_i18n::tr!("common-movement"));
        assert_eq!(sections[tabs[1].start].0, aurora_i18n::tr!("common-camera"));
        assert_eq!(sections[tabs[3].clone()].len(), 1);
        assert_eq!(sections[tabs[3].start].0, aurora_i18n::tr!("common-windows"));
        // the inventory lists' keys, all of them, in their own tab
        assert_eq!(sections[tabs[4].clone()].len(), 1);
        let inventory = &sections[tabs[4].start];
        assert_eq!(inventory.0, aurora_i18n::tr!("common-inventory"));
        assert!(inventory.1.iter().all(|(a, _)| crate::keybinds::is_inventory(*a)));
        let all = sections
            .iter()
            .flat_map(|(_, list)| list.iter().map(|(a, _)| *a))
            .filter(|a| crate::keybinds::is_inventory(*a))
            .count();
        assert_eq!(inventory.1.len(), all);
    }
}
