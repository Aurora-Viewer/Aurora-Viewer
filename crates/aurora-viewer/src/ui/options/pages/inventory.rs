//! Preferences › Inventory: the settings of the Inventory window's own
//! preferences (`ui::inventory::controls::preferences`), the order of its
//! main tabs, what follows a kept offer, and Aurora's filing of the items
//! received.

use super::super::cx::{Cx, field};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-sorting"), |cx| {
        cx.choice(
            tr!("ui-inventory-controls-display-order"),
            "",
            &field!(inventory.sort.by_date),
            &[
                (true, tr!("ui-inventory-controls-newest-first"), false),
                (false, tr!("common-by-name"), false),
            ],
        );
        cx.toggle(
            tr!("ui-inventory-controls-folders-always-by-name"),
            "",
            &field!(inventory.sort.folders_by_name),
        );
        cx.toggle(
            tr!("ui-inventory-controls-system-folders-first"),
            "",
            &field!(inventory.sort.system_first),
        );
        // the order of the Received tab, chosen in its own preferences
        cx.track(&field!(inventory.received_sort));
    });
    cx.group(tr!("common-display"), |cx| {
        cx.toggle(tr!("ui-inventory-controls-show-library"), "", &field!(inventory.show_library));
        cx.toggle(
            tr!("inventory-hover-pictures"),
            tr!("inventory-hover-pictures-help"),
            &field!(inventory.thumbnail_tooltips),
        );
        cx.toggle(tr!("ui-inventory-controls-recent-tab"), "", &field!(inventory.show_recent));
        let off = !cx.s.inventory.show_recent;
        cx.subs(off, |cx| {
            // the choice and its days and hours: too long for one column
            cx.wide().row(
                tr!("ui-inventory-recent-period"),
                tr!("ui-inventory-recent-period-help"),
                &field!(inventory.recent_hours),
                |ui, s| crate::ui::inventory::recent_period(ui, &mut s.inventory.recent_hours),
            );
        });
        cx.toggle(tr!("ui-inventory-controls-worn-tab"), "", &field!(inventory.show_worn));
        cx.toggle(tr!("ui-inventory-controls-favorites-tab"), "", &field!(inventory.show_favorites));
        cx.toggle(
            tr!("ui-inventory-controls-separate-search-in-each-tab"),
            "",
            &field!(inventory.separate_searches),
        );
    });
    cx.group(tr!("ui-options-double-click"), |cx| {
        cx.toggle(
            tr!("ui-inventory-controls-double-click-add-objects"),
            "",
            &field!(inventory.double_click_add_objects),
        );
        cx.toggle(
            tr!("ui-inventory-controls-double-click-add-clothes"),
            "",
            &field!(inventory.double_click_add_clothes),
        );
    });
    cx.group(tr!("task-transfer-preferences"), |cx| {
        cx.toggle(tr!("task-transfer-warn-move"), "", &field!(inventory.warn_task_move));
        cx.toggle(tr!("task-transfer-warn-scripted"), "", &field!(inventory.warn_scripted_task_move));
    });
    cx.group(tr!("ui-inventory-controls-also-search-in"), |cx| {
        cx.toggle(tr!("ui-inventory-controls-library"), "", &field!(inventory.search_library));
        cx.toggle(tr!("ui-inventory-controls-trash"), "", &field!(inventory.search_trash));
        cx.toggle(tr!("ui-inventory-controls-outfit-folders"), "", &field!(inventory.search_outfits));
    });
    cx.group(tr!("received-filing-title"), |cx| {
        cx.toggle(
            tr!("received-auto-accept"),
            tr!("received-auto-accept-help"),
            &field!(auto_accept_inventory),
        );
        cx.subs(!cx.s.auto_accept_inventory, |cx| {
            cx.toggle(
                tr!("received-auto-accept-chat"),
                tr!("received-auto-accept-chat-help"),
                &field!(auto_accept_inventory_chat),
            );
            cx.toggle(
                tr!("received-auto-accept-notify"),
                tr!("received-auto-accept-notify-help"),
                &field!(auto_accept_inventory_notify),
            );
        });
        // ShowInInventory ("Automatically show newly accepted items in
        // inventory", panel_preferences_privacy.xml)
        cx.toggle(
            tr!("received-show-accepted"),
            tr!("received-show-accepted-help"),
            &field!(show_accepted_in_inventory),
        );
        cx.toggle(
            tr!("received-delivery-notify"),
            tr!("received-delivery-notify-help"),
            &field!(notify_deliveries),
        );
        let changed = cx.experimental().toggle(
            tr!("received-filing-enabled"),
            tr!("received-filing-help"),
            &field!(sort_received_items),
        );
        if changed {
            cx.s.received_items_since = crate::ui::inventory::view::now();
        }
        cx.track(&field!(received_items_since));
    });
    cx.group(tr!("import-window-title"), |cx| {
        cx.choice(
            tr!("import-entry-point"),
            tr!("import-entry-point-help"),
            &field!(imports.icon_bar),
            &[(false, tr!("import-entry-window"), false), (true, tr!("import-icon-bar"), false)],
        );
    });
}
