//! Actual Ctrl-selection and pointer drags in the offline texture library's
//! inventory window; no grid data or OS input automation.
use super::*;

fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        (std::env::var("AURORA_DEMO").as_deref() == Ok("1"))
            .then(|| std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").ok())
            .flatten()
            .filter(|mode| matches!(mode.as_str(), "inventory-move" | "inventory-move-hover" | "inventory-move-rejected"))
    })
    .as_deref()
}
pub fn enabled() -> bool {
    mode().is_some()
}

pub fn setup(ctx: &egui::Context, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let Some(root) = crate::ui::texture_library::catalogue::root(inv) else {
        return;
    };
    let first = crate::demo::texture_library::first_texture();
    st.open_folder_window(root);
    let Some(w) = st.windows.last_mut() else { return };
    w.state.ui_id = w.id;
    w.state.show_original(inv, first);
    if mode() == Some("inventory-move-rejected")
        && let Some(to) = destination(inv)
    {
        prefs.protected.insert(to);
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("inventory_drag_demo_editor"), w.id));
    crate::ui::widgets::place_floater(ctx, &format!("inventory_{}", w.id), egui::pos2(40.0, 65.0), Vec2::new(550.0, 550.0));
}

fn destination(inv: &Inventory) -> Option<Uuid> {
    let item = crate::demo::texture_library::cascade_items(0)[3];
    inv.items.get(&item).map(|i| i.parent)
}

pub fn input(ctx: &egui::Context, frame: u64, inv: &Inventory, st: &InventoryUi, raw: &mut egui::RawInput) {
    let Some(mode) = mode() else { return };
    let Some(editor) = ctx.data(|d| d.get_temp::<Uuid>(egui::Id::new("inventory_drag_demo_editor"))) else {
        return;
    };
    if !st.windows.iter().any(|w| w.id == editor) {
        return;
    }
    let items = crate::demo::texture_library::cascade_items(0);
    let Some(to) = destination(inv) else { return };
    let row = |id| {
        ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("inventory_drag_row", editor, id))))
            .map(|r| r.min + Vec2::new(25.0, r.height() * 0.5))
    };
    let release = if mode == "inventory-move-hover" { 840 } else { 740 };
    let (id, pressed, ctrl) = match frame {
        620 => (items[0], Some(true), false),
        621 => (items[0], Some(false), false),
        640 => (items[1], Some(true), true),
        641 => (items[1], Some(false), true),
        660 => (items[0], Some(true), false),
        662 => (items[0], None, false),
        670 => (to, None, false),
        n if n == release => (to, Some(false), false),
        _ => return,
    };
    let Some(mut pos) = row(id) else { return };
    if frame == 662 {
        pos.x += 50.0;
    }
    let modifiers = egui::Modifiers {
        ctrl,
        command: ctrl,
        ..Default::default()
    };
    raw.events.push(egui::Event::ModifiersChanged(modifiers));
    raw.events.push(egui::Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
}
