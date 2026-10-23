//! Preferences › Shortcuts: every rebindable action, two bindings each,
//! one tab per section of `keybinds::sections` (the small ones together
//! under "Other"). The search of the window finds an action by its name,
//! its section or one of its keys.

use super::super::binding_cell;
use super::super::cx::{Custom, Cx};
use crate::keybinds::Action;
use crate::settings::Settings;

/// The sections with a tab of their own; the others share the last one.
const OWN_TABS: usize = 4;

pub fn tabs() -> Vec<&'static str> {
    let sections = crate::keybinds::sections();
    let mut tabs: Vec<&'static str> = sections.iter().take(OWN_TABS).map(|(name, _)| *name).collect();
    if sections.len() > OWN_TABS {
        tabs.push(aurora_i18n::tr!("ui-options-other-shortcuts"));
    }
    tabs
}

pub fn page(cx: &mut Cx, tab: usize) {
    let p = cx.p;
    let sections = crate::keybinds::sections();
    let shown: Vec<_> = if tab < OWN_TABS {
        sections.iter().skip(tab).take(1).collect()
    } else {
        sections.iter().skip(OWN_TABS).collect()
    };
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
                cx.wide().row_with(label, "", &t, |ui, s, st, _| {
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
