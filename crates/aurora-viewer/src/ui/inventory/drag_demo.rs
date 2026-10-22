//! Actual Ctrl-selection and pointer drags in the offline texture library's
//! inventory window; no grid data or OS input automation.
use super::*;

fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        (std::env::var("AURORA_DEMO").as_deref() == Ok("1"))
            .then(|| std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").ok())
            .flatten()
            .filter(|mode| {
                matches!(
                    mode.as_str(),
                    "inventory-move" | "inventory-move-hover" | "inventory-move-rejected" | "inventory-move-scroll" | "inventory-move-fold"
                )
            })
    })
    .as_deref()
}
pub fn enabled() -> bool {
    mode().is_some()
}

fn far_folder(n: u128) -> Uuid {
    Uuid::from_u128(0xde40_a170_0000_0000_0000_0000_0000_9000 + n)
}

pub fn setup(ctx: &egui::Context, inv: &mut Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let Some(root) = crate::ui::texture_library::catalogue::root(inv) else {
        return;
    };
    let first = crate::demo::texture_library::first_texture();
    if mode() == Some("inventory-move-scroll") {
        inv.created_folders(
            (0..81)
                .map(|n| aurora_net::inventory::InvFolder {
                    id: far_folder(n),
                    parent: root,
                    name: if n == 80 {
                        format!("ZZ {}", aurora_i18n::tr!("texture-library-demo-stone"))
                    } else {
                        format!("Z {} {n:02}", aurora_i18n::tr!("texture-library-demo-sources"))
                    },
                    type_default: -1,
                    ..Default::default()
                })
                .collect(),
        );
        for n in 0..81 {
            if let Some(folder) = inv.folders.get_mut(&far_folder(n)) {
                folder.state = FetchState::Fetched;
            }
        }
    }
    st.open_folder_window(root);
    let Some(w) = st.windows.last_mut() else { return };
    w.state.ui_id = w.id;
    w.state.show_original(inv, first);
    if mode() == Some("inventory-move-rejected")
        && let Some(to) = destination(inv)
    {
        prefs.protected.insert(to);
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("inventory_drag_demo_editor"), w.id));
    crate::ui::widgets::place_floater(ctx, &format!("inventory_{}", w.id), egui::pos2(40.0, 65.0), Vec2::new(550.0, 550.0));
}

fn destination(inv: &Inventory) -> Option<Uuid> {
    if mode() == Some("inventory-move-scroll") {
        return Some(far_folder(80));
    }
    let item = crate::demo::texture_library::cascade_items(0)[3];
    inv.items.get(&item).map(|i| i.parent)
}

pub fn input(ctx: &egui::Context, frame: u64, inv: &Inventory, st: &InventoryUi, raw: &mut egui::RawInput) {
    let Some(mode) = mode() else { return };
    let Some(editor) = ctx.data(|d| d.get_temp::<Uuid>(egui::Id::unique("inventory_drag_demo_editor"))) else {
        return;
    };
    if !st.windows.iter().any(|w| w.id == editor) {
        return;
    }
    let items = crate::demo::texture_library::cascade_items(0);
    let Some(to) = destination(inv) else { return };
    let row = |id| {
        ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", editor, id))))
            .map(|r| r.min + Vec2::new(25.0, r.height() * 0.5))
    };
    let release = match mode {
        "inventory-move-hover" => 840,
        "inventory-move-fold" => 950,
        "inventory-move-scroll" => 1050,
        _ => 740,
    };
    let (id, pressed, ctrl) = match frame {
        620 => (items[0], Some(true), false),
        621 => (items[0], Some(false), false),
        640 => (items[1], Some(true), true),
        641 => (items[1], Some(false), true),
        660 => (items[0], Some(true), false),
        662 => (items[0], None, false),
        670 => (to, None, false),
        760 if mode == "inventory-move-fold" => (inv.items[&items[0]].parent, None, false),
        780 if mode == "inventory-move-fold" => (to, None, false),
        950 if mode == "inventory-move-scroll" => (to, None, false),
        n if n == release => (to, Some(false), false),
        _ => return,
    };
    let pos = if frame == 670 && mode == "inventory-move-scroll" {
        ctx.data(|d| d.get_temp::<(egui::Rect, egui::Id)>(egui::Id::unique(("inventory_drag_viewport", editor))))
            .map(|(r, _)| egui::pos2(r.center().x, r.bottom() + 25.0))
    } else {
        row(id)
    };
    let Some(mut pos) = pos else { return };
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
