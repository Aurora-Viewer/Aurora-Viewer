//! "Create Landmark", port of LLFloaterCreateLandmark
//! (indra/newview/llfloatercreatelandmark.cpp) with
//! LLLandmarkActions::createLandmarkHere and landmarkAlreadyExists
//! (indra/newview/lllandmarkactions.cpp) and
//! LLPanelLandmarkInfo::getFullFolderName / collectLandmarkFolders
//! (indra/newview/llpanellandmarkinfo.cpp), originally LGPL 2.1.
//!
//! As in Firestorm, opening the window creates the landmark at once
//! (CreateInventoryItem, named after the parcel, in the Landmarks folder or
//! the one asked for); OK then renames it, gives it its notes and moves it to
//! the chosen folder, Cancel deletes it.
//!
//! The navigation bar's star (LLLocationInputCtrl's add landmark button,
//! LLLandmarkActions::hasParcelLandmark) asks here whether a landmark of the
//! Favorites or Landmarks folders lies on the agent's parcel. Unlike the
//! Legacy Viewer, World and Places allow repeated creation on that parcel,
//! as requested by the user; the lookup only controls the star.
//! Aurora's star adds a quick menu (Favorites / Landmarks / more options) and
//! opens a found landmark in this window, in its edit form.

use super::widgets::{self, Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use crate::world::inventory::Inventory;
use crate::world::landmarks::Landmarks;
use aurora_llsd::Llsd;
use aurora_net::inventory::operations::{Mutation, NewItem, Operation};
use aurora_net::{NetCommand, ParcelInfo};
use egui::{RichText, Vec2};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

pub const WINDOW: &str = "create_landmark";
/// FT_LANDMARK and FT_FAVORITE.
pub const FOLDER_LANDMARKS: i32 = 3;
pub const FOLDER_FAVORITES: i32 = 23;

/// The landmark found on the agent's parcel (the lit star).
#[derive(Debug, Clone, PartialEq)]
pub struct ParcelLandmark {
    pub item: Uuid,
    pub name: String,
    /// In Favorites rather than in Landmarks.
    pub favorite: bool,
}

/// AURORA_DEMO_LANDMARK=create|favorite|landmark: the Create Landmark
/// window, or a landmark of the plaza already in Favorites / Landmarks (the
/// star lit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DemoStart {
    Window,
    Quick(i32),
}

impl DemoStart {
    pub fn from_env() -> Option<Self> {
        match std::env::var("AURORA_DEMO_LANDMARK").ok()?.as_str() {
            "create" => Some(Self::Window),
            "favorite" | "favorites" => Some(Self::Quick(FOLDER_FAVORITES)),
            "landmark" | "landmarks" => Some(Self::Quick(FOLDER_LANDMARKS)),
            _ => None,
        }
    }
}

/// Cache key of the parcel landmark: inventory generation, landmark
/// assets version, agent region and a hash of the parcel's cells.
type HereKey = (u64, u64, Option<(u32, u32)>, u64);

#[derive(Default)]
pub struct CreateLandmarkUi {
    pub open: bool,
    /// AURORA_DEMO_LANDMARK: what to do once the agent stands on a parcel.
    pub demo_pending: Option<DemoStart>,
    /// CreateInventoryItem sent by the window, its item not arrived yet.
    waiting: bool,
    /// Landmarks made from the star's menu, not arrived yet.
    quick: usize,
    /// Editing a landmark found by the star (LLPanelLandmarkInfo in edit
    /// mode) rather than creating one: Cancel keeps it, Delete removes it.
    editing: bool,
    /// The landmark being made or edited, and the folder it was in.
    item: Option<(Uuid, Uuid)>,
    title: String,
    notes: String,
    folder: Uuid,
    /// "Create new folder" (notification CreateLandmarkFolder): the name typed.
    new_folder: Option<String>,
    /// Inventory changes sent by the window: their replies update the
    /// inventory like any other.
    pub edits: HashSet<Uuid>,
    here: Option<(HereKey, Option<ParcelLandmark>)>,
}

/// LLFloaterCreateLandmark::setLandmarkInfo: the parcel's name, or "Region
/// (x, y, z)" for a parcel without one.
pub fn default_title(parcel: &str, region: &str, pos: glam::Vec3) -> String {
    if parcel.is_empty() {
        format!("{region} ({}, {}, {})", pos.x.round(), pos.y.round(), pos.z.round())
    } else {
        parcel.to_owned()
    }
}

/// LLPanelLandmarkInfo::getFullFolderName: the path of a folder from the one
/// just below the root ("My Inventory" left out).
fn full_name(inv: &Inventory, id: Uuid) -> String {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut at = id;
    while seen.insert(at) {
        let Some(f) = inv.folders.get(&at) else { break };
        parts.push(f.info.name.as_str());
        if f.info.parent == inv.root || f.info.parent.is_nil() {
            break;
        }
        at = f.info.parent;
    }
    parts.reverse();
    parts.join("/")
}

/// populateFoldersList: the Landmarks folder, the favorites bar (FIRE-31031
/// puts it second), then the folders under Landmarks sorted by full name.
pub fn folders(inv: &Inventory) -> Vec<(Uuid, String)> {
    let mut out = Vec::new();
    let landmarks = crate::world::appearance::system_folder(inv, FOLDER_LANDMARKS);
    if let Some(id) = landmarks {
        out.push((id, full_name(inv, id)));
    }
    if let Some(id) = crate::world::appearance::system_folder(inv, FOLDER_FAVORITES) {
        out.push((id, aurora_i18n::tr!("landmark-create-favorites-bar").to_owned()));
    }
    let mut below = Vec::new();
    let mut stack: Vec<Uuid> = landmarks
        .and_then(|id| inv.folders.get(&id))
        .map(|f| f.children.clone())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if let Some(f) = inv.folders.get(&id).filter(|_| seen.insert(id)) {
            stack.extend(f.children.iter().copied());
            below.push((id, full_name(inv, id)));
        }
    }
    below.sort_by(|a, b| a.1.cmp(&b.1));
    out.extend(below);
    out
}

/// onSaveClicked: rename and describe the landmark when the title is not
/// empty and something changed, and move it to `folder` when it is another.
pub fn save_change(inv: &Inventory, item: Uuid, title: &str, notes: &str, folder: Uuid) -> Option<Mutation> {
    let it = inv.items.get(&item)?;
    let (title, notes) = (title.trim(), notes.trim());
    let mut body = Llsd::new_map();
    if let Ok(name) = crate::world::inventory::actions::clean_name(title)
        && (it.name != name || it.desc != notes)
    {
        body.insert("name", name);
        body.insert("desc", notes);
    }
    let moved = !folder.is_nil() && folder != it.parent;
    if moved {
        body.insert("parent_id", folder);
    }
    if body.as_map().is_none_or(|m| m.is_empty()) {
        return None;
    }
    Some(Mutation {
        operations: vec![Operation::Patch {
            id: item,
            folder: false,
            body,
        }],
        refresh: if moved { vec![it.parent, folder] } else { vec![it.parent] },
        ..Default::default()
    })
}

/// onCancelClicked: remove_inventory_item.
fn delete(inv: &Inventory, item: Uuid) -> Option<Mutation> {
    let parent = inv.items.get(&item)?.parent;
    Some(Mutation {
        operations: vec![Operation::Delete { id: item, folder: false }],
        refresh: vec![parent],
        removed: vec![item],
        ..Default::default()
    })
}

impl CreateLandmarkUi {
    /// LLLandmarkActions::hasParcelLandmark (LLFirstAgentParcelLandmark): the
    /// first landmark of the Favorites or Landmarks folders lying on the
    /// agent's parcel. Kept until the inventory, the landmark assets, the
    /// region or the parcel change (teleports and parcel crossings included).
    pub fn parcel_landmark(
        &mut self,
        inv: &Inventory,
        landmarks: &mut Landmarks,
        parcel: Option<&Arc<ParcelInfo>>,
        origin: Option<(u32, u32)>,
    ) -> Option<ParcelLandmark> {
        // the parcel by its cells (an Arc's address may be reused by the next one)
        let cells = parcel.map_or(0, |p| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            p.bitmap.hash(&mut h);
            h.finish()
        });
        let key = (inv.generation, landmarks.version, origin, cells);
        if let Some((k, found)) = &self.here
            && *k == key
        {
            return found.clone();
        }
        let found = match (parcel, origin) {
            (Some(parcel), Some(origin)) => landmarks
                .find(inv, |g| crate::world::landmarks::in_parcel(&parcel.bitmap, origin, g))
                .and_then(|id| inv.items.get(&id))
                .map(|it| ParcelLandmark {
                    item: it.id,
                    name: it.name.clone(),
                    favorite: crate::world::inventory::actions::in_type(inv, it.id, FOLDER_FAVORITES),
                }),
            _ => None,
        };
        // asking for an asset does not bump the version: the key holds
        // until it arrives
        self.here = Some((key, found.clone()));
        found
    }

    /// The star's quick entries "Add to Favorites" / "Add to Landmarks":
    /// the landmark is made at once in that folder, named like the window's
    /// title (createLandmarkHere without the window).
    pub fn quick(&mut self, world: &World, folder_kind: i32) -> Option<NetCommand> {
        if world.rlva.has("showloc") {
            return None;
        }
        let parent = crate::world::appearance::system_folder(&world.inventory, folder_kind)?;
        if world.main().is_none() || world.parcel.is_none() {
            log::warn!("landmark not created: no agent region or parcel yet");
            return None;
        }
        self.quick += 1;
        Some(NetCommand::CreateInventoryItem {
            parent,
            kind: NewItem::Landmark,
            name: default_title(&world.parcel_name, &world.region_name(), world.agent.position),
            permissions: None,
        })
    }

    /// The lit star: the landmark of this parcel in the window's edit form
    /// (LLPanelLandmarkInfo: title, folder, notes), with Delete.
    pub fn open_edit(&mut self, inv: &Inventory, item: Uuid) {
        let Some(it) = inv.items.get(&item) else { return };
        self.open = true;
        self.editing = true;
        self.waiting = false;
        self.new_folder = None;
        self.item = Some((it.id, it.parent));
        self.title = it.name.clone();
        self.notes = it.desc.clone();
        self.folder = it.parent;
    }

    /// LLFloaterReg::showInstance("add_landmark", dest_folder): open the
    /// window and create the landmark (onOpen, setLandmarkInfo). Already open:
    /// it is only brought back, without a second landmark.
    pub fn open(&mut self, world: &World, dest: Option<Uuid>) -> Option<NetCommand> {
        if world.rlva.has("showloc") {
            return None;
        }
        if self.open {
            return None;
        }
        let inv = &world.inventory;
        self.open = true;
        self.editing = false;
        self.item = None;
        self.waiting = false;
        self.new_folder = None;
        self.notes.clear();
        self.title = default_title(&world.parcel_name, &world.region_name(), world.agent.position);
        self.folder = dest
            .filter(|d| inv.folders.contains_key(d))
            .or_else(|| crate::world::appearance::system_folder(inv, FOLDER_LANDMARKS))
            .unwrap_or(inv.root);
        // createLandmarkHere: nothing without a region and a parcel
        if world.main().is_none() || world.parcel.is_none() {
            log::warn!("landmark not created: no agent region or parcel yet");
            return None;
        }
        self.waiting = true;
        Some(NetCommand::CreateInventoryItem {
            parent: self.folder,
            kind: NewItem::Landmark,
            name: world.parcel_name.clone(),
            permissions: None,
        })
    }

    pub fn apply_rlva_privacy(&mut self, hidden: bool) {
        if hidden && !self.editing {
            self.open = false;
        }
    }

    /// LLLandmarksInventoryObserver::checkCreated: the landmark the window
    /// asked for has arrived. True when it is the window's: the inventory
    /// then neither selects nor renames it (highlight_offered_object).
    pub fn created(&mut self, id: Uuid, parent: Uuid, asset_type: i32) -> bool {
        if asset_type != 3 {
            return false;
        }
        if !self.waiting {
            // one made from the star's menu: nothing more to do with it
            return self.quick.checked_sub(1).map(|n| self.quick = n).is_some();
        }
        self.waiting = false;
        if self.open {
            self.item = Some((id, parent));
        }
        true
    }

    /// folderCreatedCallback: the new folder stays selected under the id
    /// the server gave it.
    pub fn folder_created(&mut self, created: &std::collections::HashMap<Uuid, Uuid>) {
        if let Some(id) = created.get(&self.folder) {
            self.folder = *id;
        }
    }

    fn edit(&mut self, change: Option<Mutation>, out: &mut Vec<NetCommand>) {
        if let Some(change) = change {
            let request = Uuid::new_v4();
            self.edits.insert(request);
            out.push(NetCommand::EditInventory { request, change });
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, inv: &Inventory) -> Vec<NetCommand> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        // updateItem: the landmark removed or moved elsewhere in the
        // inventory ends the window
        if let Some((id, parent)) = self.item
            && inv.items.get(&id).is_none_or(|it| it.parent != parent)
        {
            self.open = false;
            self.item = None;
            return out;
        }
        let screen = ctx.content_rect();
        let mut open = true;
        let mut ok = false;
        let mut cancel = false;
        let mut remove = false;
        let mut list = folders(inv);
        // an edited landmark may sit in a folder of its own (a Favorites
        // subfolder): it stays selectable
        if !list.iter().any(|(id, _)| *id == self.folder) && inv.folders.contains_key(&self.folder) {
            list.push((self.folder, full_name(inv, self.folder)));
        }
        let editing = self.editing;
        Floater::new(
            WINDOW,
            if editing {
                aurora_i18n::tr!("landmark-edit-title")
            } else {
                aurora_i18n::tr!("ui-places-create-landmark")
            },
            egui::pos2(screen.center().x - 180.0, screen.center().y - 170.0),
            Vec2::new(360.0, 340.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let label = |ui: &mut egui::Ui, text: &str| {
                ui.label(RichText::new(text).size(12.0).color(p.muted));
            };
            ui.label(
                RichText::new(aurora_i18n::tr!("landmark-create-details"))
                    .size(14.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(8.0);
            label(ui, aurora_i18n::tr!("landmark-create-name"));
            ui.add(
                egui::TextEdit::singleline(&mut self.title)
                    .char_limit(63)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            label(ui, aurora_i18n::tr!("landmark-create-folder"));
            let selected = list.iter().find(|(id, _)| *id == self.folder).map_or("", |(_, n)| n.as_str());
            egui::ComboBox::from_id_salt("create_landmark_folder")
                .selected_text(selected)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (id, name) in &list {
                        ui.selectable_value(&mut self.folder, *id, name);
                    }
                });
            let link = ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("landmark-create-new-folder"))
                        .size(12.0)
                        .color(p.violet),
                )
                .sense(egui::Sense::click()),
            );
            if link.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if link.clicked() {
                self.new_folder = Some(String::new());
            }
            ui.add_space(6.0);
            label(ui, aurora_i18n::tr!("landmark-create-notes"));
            let mut notes = egui::TextEdit::multiline(&mut self.notes)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .show(ui);
            // spell checked, as floater_create_landmark.xml
            crate::ui::spelling::field(ui, p, &mut notes, &mut self.notes, false);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if editing {
                    remove = widgets::action_button(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("common-delete"),
                        widgets::ActionKind::Danger,
                        Vec2::new(0.0, 24.0),
                    )
                    .clicked();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
                    ok = widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(90.0, 24.0)).clicked();
                });
            });
        });
        if let Some(name) = self.new_folder.as_mut() {
            let mut done = None;
            widgets::Modal::new(egui::Id::unique("create_landmark_folder_name")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("landmark-create-folder-prompt"))
                        .size(12.5)
                        .color(p.ink),
                );
                ui.add_space(6.0);
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        done = Some(false);
                    }
                    if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(80.0, 24.0)).clicked() {
                        done = Some(true);
                    }
                });
            });
            if ui_escape(ctx) {
                done = Some(false);
            }
            if let Some(create) = done {
                let name = self.new_folder.take().unwrap_or_default();
                // createNewCategory under Landmarks, then the list shows it
                // selected (folderCreatedCallback)
                if create
                    && !name.trim().is_empty()
                    && let Some(parent) = crate::world::appearance::system_folder(inv, FOLDER_LANDMARKS)
                    && let Ok(change) = crate::world::inventory::actions::create_folder(parent, &name)
                {
                    if let Some(Operation::Folder { id, .. }) = change.operations.first() {
                        self.folder = *id;
                    }
                    self.edit(Some(change), &mut out);
                }
            }
        }
        if ok {
            if let Some((id, _)) = self.item {
                let change = save_change(inv, id, &self.title, &self.notes, self.folder);
                self.edit(change, &mut out);
            }
            self.open = false;
        } else if editing && (cancel || !open) {
            // an existing landmark: Cancel leaves it as it was
            self.open = false;
        } else if remove || cancel || !open {
            // the creation window has no close button in Firestorm: closing
            // it is Cancel
            if let Some((id, _)) = self.item {
                let change = delete(inv, id);
                self.edit(change, &mut out);
            }
            self.open = false;
        }
        if !self.open {
            self.item = None;
            self.new_folder = None;
        }
        out
    }
}

fn ui_escape(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_pressed(egui::Key::Escape))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};

    fn folder(id: u128, parent: u128, name: &str, t: i32) -> InvFolder {
        InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            type_default: t,
            version: 1,
            favorite: false,
            favorite_known: false,
            thumbnail: Uuid::nil(),
        }
    }

    fn inventory() -> Inventory {
        let mut inv = Inventory::default();
        let contents = |f: InvFolder, folders: Vec<InvFolder>, items: Vec<InvItem>| FolderContents {
            folder_id: f.id,
            owner_id: Uuid::nil(),
            version: 2,
            folders,
            items,
        };
        let root = folder(1, 0, "My Inventory", 8);
        let lms = folder(10, 1, "Landmarks", 3);
        let fav = folder(30, 1, "Favorites", 23);
        let trips = folder(11, 10, "Trips", -1);
        let paris = folder(12, 11, "Paris", -1);
        let art = folder(13, 10, "Art", -1);
        inv.root = root.id;
        inv.apply(vec![contents(root, vec![lms.clone(), fav.clone()], Vec::new())]);
        inv.apply(vec![contents(lms, vec![trips.clone(), art.clone()], Vec::new())]);
        inv.apply(vec![contents(trips, vec![paris.clone()], Vec::new())]);
        inv.apply(vec![contents(paris, Vec::new(), Vec::new())]);
        inv.apply(vec![contents(art, Vec::new(), Vec::new())]);
        inv.apply(vec![contents(
            fav,
            Vec::new(),
            vec![InvItem {
                id: Uuid::from_u128(100),
                parent: Uuid::from_u128(30),
                name: "Plage".into(),
                desc: String::new(),
                asset_id: Uuid::from_u128(1100),
                asset_type: 3,
                inv_type: 3,
                ..Default::default()
            }],
        )]);
        inv
    }

    #[test]
    fn title_is_the_parcel_or_the_region_and_position() {
        assert_eq!(default_title("Lagune", "Aurora", glam::Vec3::new(1.0, 2.0, 3.0)), "Lagune");
        assert_eq!(
            default_title("", "Aurora", glam::Vec3::new(128.4, 64.6, 22.0)),
            "Aurora (128, 65, 22)"
        );
    }

    #[test]
    fn folders_list_landmarks_favorites_then_subfolders_by_full_name() {
        let names: Vec<String> = folders(&inventory()).into_iter().map(|(_, n)| n).collect();
        assert_eq!(
            names,
            [
                "Landmarks",
                aurora_i18n::tr!("landmark-create-favorites-bar"),
                "Landmarks/Art",
                "Landmarks/Trips",
                "Landmarks/Trips/Paris"
            ]
        );
    }

    #[test]
    fn ok_renames_describes_and_moves_only_what_changed() {
        let inv = inventory();
        let item = Uuid::from_u128(100);
        let fav = Uuid::from_u128(30);
        assert!(save_change(&inv, item, "  Plage ", "", fav).is_none(), "nothing changed");
        let body = |m: Mutation| match &m.operations[..] {
            [Operation::Patch { body, .. }] => (body.clone(), m.refresh),
            _ => panic!("one patch"),
        };
        let (b, refresh) = body(save_change(&inv, item, "Plage", "Au soleil", fav).expect("notes changed"));
        assert_eq!(b, llsd_map! { "name" => "Plage", "desc" => "Au soleil" });
        assert_eq!(refresh, vec![fav]);
        let art = Uuid::from_u128(13);
        let (b, refresh) = body(save_change(&inv, item, "", "", art).expect("moved"));
        assert_eq!(b, llsd_map! { "parent_id" => art }, "an empty title renames nothing");
        assert_eq!(refresh, vec![fav, art]);
    }

    #[test]
    fn the_star_finds_a_landmark_on_the_agent_parcel_and_follows_the_parcel() {
        let inv = inventory();
        let region = Uuid::from_u128(0x5E61);
        let origin = (256_000, 256_000);
        let mut landmarks = Landmarks::default();
        landmarks.set_local_region(region, aurora_net::origin_to_handle(origin.0, origin.1));
        // the landmark points at (10, 41): cell x 2, y 10
        let text = format!("Landmark version 2\nregion_id {region}\nlocal_pos 10 41 22\n");
        landmarks.on_data(Uuid::from_u128(1100), Some(text.as_bytes()));
        let parcel_with = |cells: &[(usize, usize)]| {
            let mut bitmap = vec![0u8; 512];
            for (x, y) in cells {
                let i = y * 64 + x;
                bitmap[i / 8] |= 1 << (i % 8);
            }
            Arc::new(ParcelInfo {
                bitmap,
                ..Default::default()
            })
        };
        let mut ui = CreateLandmarkUi::default();
        let here = parcel_with(&[(2, 10), (3, 10)]);
        let found = ui.parcel_landmark(&inv, &mut landmarks, Some(&here), Some(origin));
        assert_eq!(
            found,
            Some(ParcelLandmark {
                item: Uuid::from_u128(100),
                name: "Plage".into(),
                favorite: true
            })
        );
        // another parcel of the same region (a crossing): not lit any more
        let next = parcel_with(&[(4, 10)]);
        assert_eq!(ui.parcel_landmark(&inv, &mut landmarks, Some(&next), Some(origin)), None);
        // another region (a teleport) with the same cells
        assert_eq!(
            ui.parcel_landmark(&inv, &mut landmarks, Some(&here), Some((512_000, 256_000))),
            None
        );
        assert_eq!(ui.parcel_landmark(&inv, &mut landmarks, None, Some(origin)), None, "no parcel yet");
        assert!(ui.parcel_landmark(&inv, &mut landmarks, Some(&here), Some(origin)).is_some());
    }

    #[test]
    fn landmarks_made_from_the_star_stay_out_of_the_window() {
        let mut ui = CreateLandmarkUi {
            quick: 1,
            ..Default::default()
        };
        assert!(ui.created(Uuid::from_u128(6), Uuid::from_u128(30), 3));
        assert_eq!(ui.item, None);
        assert!(!ui.created(Uuid::from_u128(7), Uuid::from_u128(30), 3), "only the one asked for");
    }

    #[test]
    fn only_the_awaited_landmark_belongs_to_the_window() {
        let mut ui = CreateLandmarkUi {
            open: true,
            waiting: true,
            ..Default::default()
        };
        assert!(!ui.created(Uuid::from_u128(5), Uuid::nil(), 7), "a notecard");
        assert!(ui.created(Uuid::from_u128(6), Uuid::from_u128(10), 3));
        assert_eq!(ui.item, Some((Uuid::from_u128(6), Uuid::from_u128(10))));
        assert!(!ui.created(Uuid::from_u128(7), Uuid::nil(), 3), "only one");
    }
}
