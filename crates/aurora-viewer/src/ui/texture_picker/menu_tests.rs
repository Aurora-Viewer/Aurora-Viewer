//! The inventory's right-click menu in the texture and material picker
//! (#4896): LLFloaterTexturePicker's inventory panel has the inventory's
//! menus (`inventory::picker_menu`).

use super::tests::PickerUi;
use super::*;
use crate::ui::inventory::{InvAction, InventoryPreferences, InventoryUi, picker_menu};
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::InvFolder;

const ROOT: Uuid = Uuid::from_u128(0x4896_0001);
const FOLDER: Uuid = Uuid::from_u128(0x4896_0002);
const LIBRARY: Uuid = Uuid::from_u128(0x4896_0003);
const AGENT: Uuid = Uuid::from_u128(0x4896_00a6);
const OTHER: Uuid = Uuid::from_u128(0x4896_00b7);
const LIBRARY_OWNER: Uuid = Uuid::from_u128(0x4896_00c8);

fn asset_of(item: Uuid) -> Uuid {
    Uuid::from_u128(item.as_u128() + 0x1000)
}

/// An item of the fixture: its name, folder, owner and rights.
struct Item {
    id: Uuid,
    name: &'static str,
    parent: Uuid,
    owner: Uuid,
    group_owned: bool,
    owner_mask: u32,
    everyone_mask: u32,
    material: bool,
}

impl Item {
    fn texture(id: u128, name: &'static str) -> Self {
        Self {
            id: Uuid::from_u128(id),
            name,
            parent: FOLDER,
            owner: AGENT,
            group_owned: false,
            owner_mask: actions::FULL,
            everyone_mask: 0,
            material: false,
        }
    }
}

/// My Inventory with a "Project textures" folder, and the Library.
fn world(items: &[Item]) -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = AGENT;
    let inv = &mut world.inventory;
    inv.root = ROOT;
    inv.owner = AGENT;
    inv.lib_root = LIBRARY;
    inv.lib_owner = LIBRARY_OWNER;
    for (id, parent, name, type_default, library) in [
        (ROOT, Uuid::nil(), "My Inventory", 8, false),
        (FOLDER, ROOT, "Project textures", -1, false),
        (LIBRARY, Uuid::nil(), "Library", 8, true),
    ] {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default,
                    version: 1,
                    ..Default::default()
                },
                children: if id == ROOT { vec![FOLDER] } else { Vec::new() },
                items: items.iter().filter(|it| it.parent == id).map(|it| it.id).collect(),
                state: FetchState::Fetched,
                library,
            },
        );
    }
    for it in items {
        inv.items.insert(
            it.id,
            aurora_net::inventory::InvItem {
                id: it.id,
                parent: it.parent,
                name: it.name.into(),
                asset_id: asset_of(it.id),
                asset_type: if it.material { 57 } else { 0 },
                inv_type: if it.material { INV_MATERIAL } else { INV_TEXTURE },
                owner: it.owner,
                creator: AGENT,
                group_owned: it.group_owned,
                owner_mask: it.owner_mask,
                base_mask: actions::FULL,
                everyone_mask: it.everyone_mask,
                ..Default::default()
            },
        );
    }
    inv.generation += 1;
    world
}

/// The picker drawn with the inventory's pass after it, as in the viewer.
struct Bench {
    picker: TexturePicker,
    world: World,
    main: InventoryUi,
    prefs: InventoryPreferences,
    actions: Vec<InvAction>,
    changes: Vec<Uuid>,
}

impl Bench {
    fn new(items: &[Item]) -> Self {
        Self {
            picker: TexturePicker::default(),
            world: world(items),
            main: InventoryUi::default(),
            prefs: InventoryPreferences::default(),
            actions: Vec::new(),
            changes: Vec::new(),
        }
    }

    fn draw(&mut self, ctx: &egui::Context, p: &Palette) {
        if let Some(value) = self.picker.show_live(ctx, p, &mut self.world, &HashMap::new()) {
            self.changes.push(value);
        }
        self.actions.extend(crate::ui::inventory::show(
            ctx,
            p,
            &crate::ui::icons::Icons::default(),
            &mut self.world,
            &mut self.main,
            None,
            &mut self.prefs,
            &mut false,
            false,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &Default::default(),
            false,
        ));
    }
}

const PROPERTIES: &str = "common-properties";

fn label(key: &str) -> &'static str {
    match key {
        PROPERTIES => aurora_i18n::tr!("common-properties"),
        "rename" => aurora_i18n::tr!("common-rename"),
        "copy-name" => aurora_i18n::tr!("inventory-copy-name"),
        "asset-uuid" => aurora_i18n::tr!("ui-inventory-context-copy-object-uuid"),
        "show-main" => aurora_i18n::tr!("inventory-show-main"),
        _ => unreachable!("{key}"),
    }
}

#[test]
fn a_right_click_on_a_texture_or_material_row_opens_the_inventory_menu() {
    for material in [false, true] {
        let item = Item {
            material,
            ..Item::texture(0x10, if material { "Tiled material" } else { "Brick wall" })
        };
        let mut bench = Bench::new(&[item]);
        if material {
            bench.picker.open_materials("Material", asset_of(Uuid::from_u128(0x10)));
        } else {
            bench
                .picker
                .open_with_defaults("Texture", asset_of(Uuid::from_u128(0x10)), None, true);
        }
        let mut ui = PickerUi::new();
        ui.settle(|ctx, p| bench.draw(ctx, p));
        assert!(!ui.has(label(PROPERTIES)), "no menu before the right-click");
        let name = if material { "Tiled material" } else { "Brick wall" };
        ui.right_click(name, |ctx, p| bench.draw(ctx, p));
        // keyed on the row's item, the menu stays open over the next passes
        ui.settle(|ctx, p| bench.draw(ctx, p));
        for key in [PROPERTIES, "rename", "copy-name", "asset-uuid", "show-main"] {
            assert!(ui.has(label(key)), "{name}: {key} in the menu");
        }
        // Copy Name, as in the issue's recording: the item's name is copied
        ui.click(label("copy-name"), |ctx, p| bench.draw(ctx, p));
        assert_eq!(ui.copied().last().map(String::as_str), Some(name));
        ui.settle(|ctx, p| bench.draw(ctx, p));
        assert!(!ui.has(label(PROPERTIES)), "a command closes the menu");
    }
}

#[test]
fn the_menu_commands_reach_the_inventory_dialogs_and_its_shared_clipboard() {
    let brick = Uuid::from_u128(0x10);
    let mut bench = Bench::new(&[Item::texture(0x10, "Brick wall")]);
    bench.picker.open_with_defaults("Texture", asset_of(brick), None, true);
    let mut ui = PickerUi::new();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    // Copy: the one clipboard of every inventory window (LLClipboard)
    ui.right_click("Brick wall", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    // Copy is an icon of the menu's top row (slot 4, `context_catalog`)
    let copy = picker_menu::menu_rect(ui.ctx(), "slot:top:4").expect("the Copy icon");
    ui.click_at(copy.center(), |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert_eq!(bench.main.clipboard, vec![brick]);
    assert!(!bench.main.cut);
    // a later copy elsewhere is not undone by the picker's older one
    let other = Uuid::from_u128(0x77);
    bench.main.clipboard = vec![other];
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert_eq!(bench.main.clipboard, vec![other]);
    // Properties: the inventory's own window, drawn with its dialogs
    ui.right_click("Brick wall", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.click(label(PROPERTIES), |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert_eq!(picker_menu::properties(ui.ctx(), &bench.main), Some(brick));
}

#[test]
fn rename_edits_the_name_in_the_picker_row_and_reaches_the_inventory() {
    let brick = Uuid::from_u128(0x10);
    let mut bench = Bench::new(&[Item::texture(0x10, "Brick wall")]);
    bench.picker.open_with_defaults("Texture", asset_of(brick), None, true);
    let mut ui = PickerUi::new();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.right_click("Brick wall", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.click(label("rename"), |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.ctx().memory(|m| m.focused()).is_some(), "the row's name field has the keyboard");
    bench.actions.clear();
    ui.frame(vec![egui::Event::Text("Red brick".into())], |ctx, p| bench.draw(ctx, p));
    ui.frame(
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        |ctx, p| bench.draw(ctx, p),
    );
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(
        bench.actions.iter().any(|action| matches!(action, InvAction::Edit(_))),
        "the new name goes to the inventory: {:?}",
        bench.actions.len()
    );
    assert!(picker_menu::pending_actions(ui.ctx()).is_empty());
    // the app answers in the main window's message (here a refusal, once
    // the change came back): the picker's status line shows it too
    bench.main.pending = Some(Uuid::from_u128(0x99));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    bench.main.pending = None;
    bench.main.message = "The name was refused".into();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.has("The name was refused"));
}

#[test]
fn copy_asset_uuid_through_the_picker_follows_the_inventory_rule() {
    // LLViewerInventoryItem::getProtectedAssetUUID: full permissions only
    // (`context::copyable_asset_uuid`, `actions::held`).
    let cases = [
        ("own, full", AGENT, false, actions::FULL, 0, FOLDER, true),
        ("own, no copy", AGENT, false, actions::FULL & !actions::COPY, 0, FOLDER, false),
        ("own, no modify", AGENT, false, actions::FULL & !actions::MODIFY, 0, FOLDER, false),
        (
            "own, no transfer",
            AGENT,
            false,
            actions::FULL & !actions::TRANSFER,
            0,
            FOLDER,
            false,
        ),
        ("someone else's", OTHER, false, actions::FULL, actions::COPY, FOLDER, false),
        ("group-owned", AGENT, true, actions::FULL, actions::COPY, FOLDER, false),
        ("Library, full", LIBRARY_OWNER, false, actions::FULL, actions::FULL, LIBRARY, true),
        (
            "Library, copy only",
            LIBRARY_OWNER,
            false,
            actions::FULL,
            actions::COPY,
            LIBRARY,
            false,
        ),
    ];
    for (case, owner, group_owned, owner_mask, everyone_mask, parent, copies) in cases {
        let id = Uuid::from_u128(0x20);
        let mut bench = Bench::new(&[Item {
            parent,
            owner,
            group_owned,
            owner_mask,
            everyone_mask,
            ..Item::texture(0x20, "Case texture")
        }]);
        bench.picker.open_with_defaults("Texture", asset_of(id), None, true);
        let mut ui = PickerUi::new();
        ui.settle(|ctx, p| bench.draw(ctx, p));
        ui.right_click("Case texture", |ctx, p| bench.draw(ctx, p));
        ui.settle(|ctx, p| bench.draw(ctx, p));
        assert!(ui.has(label(PROPERTIES)), "{case}: the menu is open");
        // shown greyed out when not allowed: its click copies nothing
        assert!(ui.has(label("asset-uuid")), "{case}: the entry");
        ui.click(label("asset-uuid"), |ctx, p| bench.draw(ctx, p));
        let asset = asset_of(id).to_string();
        assert_eq!(ui.copied().contains(&asset), copies, "{case}");
    }
}

#[test]
fn a_right_click_selects_a_row_as_a_click_does_and_leaves_the_rest() {
    let (brick, granite) = (Uuid::from_u128(0x10), Uuid::from_u128(0x11));
    let mut bench = Bench::new(&[
        Item::texture(0x10, "Brick wall"),
        Item::texture(0x11, "Granite"),
        Item {
            owner_mask: actions::COPY | actions::MODIFY,
            ..Item::texture(0x12, "Sealed stone")
        },
    ]);
    let local = Uuid::from_u128(0x4896_0100);
    bench.world.local_assets.insert(local, ("Local brick".into(), false));
    bench.picker.open_with_defaults("Texture", asset_of(brick), None, true);
    bench.picker.allow_local = true;
    let mut ui = PickerUi::new();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert_eq!(bench.picker.selected_item, Some(brick));
    // LLFolderViewItem::handleRightMouseDown → setSelection → onSelectionChange
    // (user_action): selected, then applied as by a click
    ui.right_click("Granite", |ctx, p| bench.draw(ctx, p));
    assert_eq!(bench.picker.selected_item, Some(granite));
    assert_eq!(bench.picker.selected, Some(asset_of(granite)));
    assert_eq!(bench.changes, [asset_of(granite)]);
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.has(label(PROPERTIES)), "and its menu opens");
    // the selected row: nothing changes
    ui.right_click("Granite", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert_eq!(bench.changes, [asset_of(granite)]);
    // a folder: its menu, the choice and the folder as they were
    ui.right_click("Project textures", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.has(label(PROPERTIES)) || ui.has(label("rename")), "the folder's menu");
    assert!(ui.has("Brick wall"), "the folder stays open");
    assert_eq!(bench.picker.selected_item, Some(granite));
    assert_eq!(bench.changes, [asset_of(granite)]);
    // Escape closes the menu
    ui.frame(
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        |ctx, p| bench.draw(ctx, p),
    );
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(!ui.has(label("rename")), "Escape closes the menu");
    // a row greyed out for this control: no menu, no choice (the Legacy
    // Viewer's immediate filter does not list it)
    bench.picker.required_permissions = actions::COPY | actions::TRANSFER;
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.right_click("Sealed stone", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(!ui.has(label(PROPERTIES)) && !ui.has(label("rename")));
    assert_eq!(bench.picker.selected_item, Some(granite));
    bench.picker.required_permissions = 0;
    // the Local list (LLScrollListCtrl, MENU_NONE): no menu, no choice
    bench.picker.show_local_tab();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.right_click("Local brick", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(!ui.has(label(PROPERTIES)) && !ui.has(label("rename")));
    assert_eq!(bench.picker.selected, Some(asset_of(granite)));
    assert_eq!(bench.changes, [asset_of(granite)]);
}

#[test]
fn the_no_copy_alert_of_a_right_click_leaves_the_menu_usable() {
    // FSPanelFace::onTextureSelectionChanged warns of a no-copy choice
    // (LivePreviewUnavailable) whatever selected it, a right-click included.
    let mut bench = Bench::new(&[
        Item::texture(0x10, "Brick wall"),
        Item {
            owner_mask: actions::MODIFY | actions::TRANSFER,
            ..Item::texture(0x13, "No-copy stone")
        },
    ]);
    bench
        .picker
        .open_with_defaults("Texture", asset_of(Uuid::from_u128(0x10)), None, true);
    bench.picker.warn_live_preview = true;
    let mut ui = PickerUi::new();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.right_click("No-copy stone", |ctx, p| bench.draw(ctx, p));
    for _ in 0..6 {
        ui.settle(|ctx, p| bench.draw(ctx, p));
    }
    assert_eq!(bench.picker.selected_item, Some(Uuid::from_u128(0x13)));
    assert!(bench.changes.is_empty(), "a no-copy choice waits for OK");
    // the Legacy Viewer's alert is not modal: this dialog waits for the menu
    let warning = aurora_i18n::tr!("ui-texture-picker-live-preview-texture");
    assert!(!ui.has(warning), "no dialog over the open menu");
    assert!(ui.has(label("copy-name")), "the menu stays open");
    ui.click(label("copy-name"), |ctx, p| bench.draw(ctx, p));
    assert_eq!(ui.copied().last().map(String::as_str), Some("No-copy stone"));
    for _ in 0..2 {
        ui.settle(|ctx, p| bench.draw(ctx, p));
    }
    assert!(ui.has(warning), "then the alert, once the menu closed");
}

#[test]
fn a_library_row_gets_the_library_menu() {
    let id = Uuid::from_u128(0x30);
    let mut bench = Bench::new(&[Item {
        parent: LIBRARY,
        owner: LIBRARY_OWNER,
        everyone_mask: actions::FULL,
        ..Item::texture(0x30, "Library plywood")
    }]);
    bench.picker.open_with_defaults("Texture", asset_of(id), None, true);
    let mut ui = PickerUi::new();
    ui.settle(|ctx, p| bench.draw(ctx, p));
    ui.right_click("Library plywood", |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.has(label(PROPERTIES)), "the Library item's menu");
    // the Library is read-only: Rename does nothing there
    ui.click(label("rename"), |ctx, p| bench.draw(ctx, p));
    ui.settle(|ctx, p| bench.draw(ctx, p));
    assert!(ui.ctx().memory(|m| m.focused()).is_none(), "no rename field");
}
