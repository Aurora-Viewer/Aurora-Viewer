//! Inventory-backed, fixed-frame animation layers for the local pose editor.
use super::*;
use crate::{
    scene::anim::{AnimStreamer, BoundAnim},
    world::inventory::{Inventory, actions},
};
use ik::animations::{Sample, last_frame};
use std::sync::Arc;

const LIMIT: usize = 16;
struct AnimationDrag(Uuid);

pub(super) struct Row {
    id: Uuid,
    item: Option<Uuid>,
    pub asset: Uuid,
    pub name: String,
    pub frame: u32,
    pub enabled: bool,
    pub clip: Option<Arc<BoundAnim>>,
    failed: bool,
}

impl Row {
    pub fn new(asset: Uuid, name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            item: None,
            asset,
            name,
            frame: 0,
            enabled: true,
            clip: None,
            failed: false,
        }
    }
}

#[derive(Default)]
pub(super) struct Stack {
    pub rows: Vec<Row>,
    pub suppress_live: bool,
    picker: bool,
    replacement: Option<usize>,
    requested: std::collections::HashSet<Uuid>,
    out: Vec<aurora_net::NetCommand>,
    filter: String,
    cache: Option<(u64, String)>,
    candidates: Vec<(Uuid, String)>,
}

#[derive(Clone, Default, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedStack {
    pub rows: Vec<SavedRow>,
    pub suppress_live: bool,
}
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedRow {
    #[serde(default)]
    pub item: Option<Uuid>,
    /// Written to project files for full-permission items only
    /// (`SavedStack::name_assets`). A row always plays the asset of its
    /// inventory item: this field never starts a download on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<Uuid>,
    pub name: String,
    pub frame: u32,
    pub enabled: bool,
}
impl SavedStack {
    pub fn valid(&self) -> bool {
        self.rows.len() <= LIMIT
            && self
                .rows
                .iter()
                .all(|r| r.name.len() <= 256 && r.frame <= 1_000_000 && r.asset.is_none_or(|a| !a.is_nil()))
    }
    /// Keeps a row's asset id only when `shared` gives one for its item, and
    /// then the item's current one: project files name animations by
    /// inventory item.
    pub fn name_assets(&mut self, shared: impl Fn(Uuid) -> Option<Uuid>) {
        for row in &mut self.rows {
            row.asset = row.item.and_then(&shared);
        }
    }
}

/// The asset of an animation item of the agent's inventory that has full
/// permissions (copy, modify and transfer), like the UUID field of the
/// texture picker (`actions::asset_uuid_shown`).
pub(super) fn shared_asset(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    let original = inv.items.get(&actions::original(inv, id)?)?;
    if original.owner_mask & actions::FULL != actions::FULL {
        return None;
    }
    item(inv, id).map(|(asset, _)| asset)
}

/// `shared_asset` of every animation item, gathered on the main thread for
/// a background export of a project read from disk.
#[derive(Default)]
pub(super) struct SharedAssets(std::collections::HashMap<Uuid, Uuid>);
impl SharedAssets {
    pub fn new(inv: &Inventory) -> Self {
        Self(
            inv.items
                .values()
                .filter(|it| it.asset_type == 20)
                .filter_map(|it| Some((it.id, shared_asset(inv, it.id)?)))
                .collect(),
        )
    }
    pub fn get(&self, item: Uuid) -> Option<Uuid> {
        self.0.get(&item).copied()
    }
}

fn item(inv: &Inventory, id: Uuid) -> Option<(Uuid, String)> {
    let original = actions::original(inv, id)?;
    let it = inv.items.get(&original)?;
    (it.asset_type == 20 && !it.asset_id.is_nil() && !inv.in_folder_type(original, 14)).then(|| (it.asset_id, it.name.clone()))
}

impl Stack {
    pub fn add(&mut self, inv: &Inventory, id: Uuid) {
        if self.rows.len() >= LIMIT && self.replacement.is_none() {
            return;
        }
        if let Some((asset, name)) = item(inv, id) {
            let mut row = Row::new(asset, name);
            row.item = actions::original(inv, id);
            if let Some(index) = self.replacement.take().filter(|i| *i < self.rows.len()) {
                row.frame = self.rows[index].frame;
                row.enabled = self.rows[index].enabled;
                self.rows[index] = row;
            } else {
                self.rows.push(row);
            }
        }
    }
    pub fn take_requests(&mut self, owner: Uuid) -> Vec<aurora_net::NetCommand> {
        let ids: Vec<_> = self
            .rows
            .iter()
            .filter_map(|r| r.item)
            .filter(|id| self.requested.insert(*id))
            .collect();
        if !ids.is_empty() {
            self.out.push(aurora_net::NetCommand::FetchItems { items: ids, owner });
        }
        std::mem::take(&mut self.out)
    }
    pub fn tick(&mut self, streamer: &mut AnimStreamer, inv: &mut Inventory) {
        if self.picker
            && let Some(root) = actions::system(inv, 20)
        {
            actions::request_tree(inv, root);
        }
        for row in &mut self.rows {
            if let Some(id) = row.item {
                if let Some((asset, name)) = item(inv, id) {
                    if asset != row.asset {
                        row.asset = asset;
                        row.clip = None;
                    }
                    row.name = name;
                } else {
                    row.failed = true;
                    row.clip = None;
                    continue;
                }
            } else if row.clip.is_none() {
                // A row plays an animation of the agent's inventory: an asset
                // id alone (a project file from someone else) stays missing.
                // Only the clips the editor builds itself (demo) have no item.
                row.failed = true;
                continue;
            }
            if row.clip.is_none() {
                row.clip = streamer.get(&row.asset);
            }
            row.failed = row.clip.is_none() && streamer.asset_settled(&row.asset);
            if let Some(clip) = &row.clip {
                row.frame = row.frame.min(last_frame(clip.anim.duration));
            }
        }
    }
    pub fn samples(&self) -> Vec<Sample> {
        self.rows
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| {
                Some(Sample {
                    animation: r.clip.clone()?,
                    frame: r.frame,
                })
            })
            .collect()
    }
    pub(super) fn save(&self) -> SavedStack {
        SavedStack {
            suppress_live: self.suppress_live,
            rows: self
                .rows
                .iter()
                .map(|r| SavedRow {
                    item: r.item,
                    asset: (!r.asset.is_nil()).then_some(r.asset),
                    name: r.name.clone(),
                    frame: r.frame,
                    enabled: r.enabled,
                })
                .collect(),
        }
    }
    pub(super) fn restore(&mut self, saved: &SavedStack) {
        let previous = std::mem::take(&mut self.rows);
        self.suppress_live = saved.suppress_live;
        for row in &saved.rows {
            let mut restored = Row::new(row.asset.unwrap_or_default(), row.name.clone());
            restored.item = row.item;
            restored.frame = row.frame;
            restored.enabled = row.enabled;
            // Reuse a clip already playing for the same item; rows without an
            // item only find the editor's own clips (demo) this way.
            if let Some(kept) = previous
                .iter()
                .find(|r| r.item == row.item && (row.item.is_some() || r.asset == restored.asset))
                && kept.clip.is_some()
            {
                restored.asset = kept.asset;
                restored.clip = kept.clip.clone();
            }
            self.rows.push(restored);
        }
    }
    fn reorder(&mut self, index: usize, delta: isize) {
        if let Some(to) = index.checked_add_signed(delta).filter(|to| *to < self.rows.len()) {
            self.rows.swap(index, to);
        }
    }
    pub fn demo(&mut self, rig: &Rig, mode: &str) {
        use aurora_assets::{Animation, JointMotion, anim::RotKey};
        for index in 0..2 {
            let end = if index == 0 {
                Quat::from_rotation_z(0.8)
            } else {
                Quat::from_rotation_x(-0.6)
            };
            let anim = Animation {
                base_priority: 3,
                duration: 2.0,
                emote_name: String::new(),
                loop_in: 0.0,
                loop_out: 2.0,
                looping: true,
                ends_after: None,
                ease_in: 0.0,
                ease_out: 0.0,
                hand_pose: 0,
                joints: vec![JointMotion {
                    joint_name: "mHead".into(),
                    priority: -1,
                    rot_keys: vec![
                        RotKey {
                            time: 0.0,
                            rotation: Quat::IDENTITY,
                        },
                        RotKey { time: 2.0, rotation: end },
                    ],
                    pos_keys: Vec::new(),
                }],
            };
            let mut row = Row::new(
                Uuid::from_u128(0xa1170000 + index),
                aurora_i18n::text(if index == 0 {
                    "ik-edit-animation-demo-first"
                } else {
                    "ik-edit-animation-demo-second"
                })
                .to_owned(),
            );
            row.frame = if index == 0 { 15 } else { 45 };
            row.clip = Some(Arc::new(BoundAnim::bind(anim, rig)));
            self.rows.push(row);
        }
        if mode == "animations-reordered" {
            self.reorder(1, -1);
        }
        if mode == "animations-frame" {
            self.rows[0].frame = 50;
            self.rows[1].enabled = false;
        }
        if mode == "animations-neutral" {
            self.suppress_live = true;
            for row in &mut self.rows {
                row.enabled = false;
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory) {
        let response = ui
            .vertical(|ui| {
                widgets::switch_labeled(ui, p, &mut self.suppress_live, aurora_i18n::tr!("ik-edit-animation-suppress"))
                    .on_hover_text(aurora_i18n::tr!("ik-edit-animation-local"));
                let mut movement = None;
                let mut removed = None;
                let mut dragged = None;
                let mut replace = None;
                let mut rects = Vec::new();
                let count = self.rows.len();
                for (index, row) in self.rows.iter_mut().enumerate() {
                    let response = ui
                        .push_id(row.id, |ui| {
                            ui.horizontal(|ui| {
                                let drag = ui.add(egui::Label::new("≡").sense(egui::Sense::drag())).on_hover_text(format!(
                                    "{}\n{}",
                                    aurora_i18n::tr!("ik-profile-drag-animation"),
                                    aurora_i18n::tr!("ik-edit-animation-order-hint")
                                ));
                                if drag.drag_started() {
                                    egui::DragAndDrop::set_payload(ui.ctx(), AnimationDrag(row.id));
                                }
                                widgets::checkbox(ui, p, &mut row.enabled, "");
                                if widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("ik-edit-animation-up"), index > 0) {
                                    movement = Some((index, -1));
                                }
                                if widgets::icon_button(ui, p, "arrow-down", aurora_i18n::tr!("ik-edit-animation-down"), index + 1 < count)
                                {
                                    movement = Some((index, 1));
                                }
                                if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("ik-edit-animation-remove"), true) {
                                    removed = Some(index);
                                }
                                ui.add(egui::Label::new(&row.name).truncate()).on_hover_text(&row.name);
                            });
                            if let Some(clip) = &row.clip {
                                frame_controls(ui, &mut row.frame, last_frame(clip.anim.duration));
                            } else {
                                if row.failed
                                    && widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("ik-project-replace-animation"), true)
                                {
                                    replace = Some(index);
                                }
                                ui.colored_label(
                                    p.muted,
                                    aurora_i18n::text(if row.failed {
                                        "ik-edit-animation-failed"
                                    } else {
                                        "ik-edit-animation-loading"
                                    }),
                                );
                            }
                            ui.add_space(6.0);
                        })
                        .response;
                    rects.push((index, response.rect));
                }
                if let Some(payload) = egui::DragAndDrop::payload::<AnimationDrag>(ui.ctx())
                    && let Some(pointer) = ui.ctx().pointer_interact_pos()
                    && ui.input(|i| i.pointer.any_released())
                    && let Some((to, _)) = rects.iter().find(|(_, r)| r.contains(pointer))
                    && let Some(from) = self.rows.iter().position(|r| r.id == payload.0)
                {
                    dragged = Some((from, *to));
                    egui::DragAndDrop::clear_payload(ui.ctx());
                }
                if let Some((from, to)) = dragged {
                    let row = self.rows.remove(from);
                    self.rows.insert(to, row);
                }
                if let Some((index, delta)) = movement {
                    self.reorder(index, delta);
                }
                if let Some(index) = removed {
                    self.rows.remove(index);
                    self.replacement = None;
                }
                if let Some(index) = replace {
                    self.replacement = Some(index);
                    self.picker = true;
                }
                if ui
                    .add_enabled(count < LIMIT, egui::Button::new(aurora_i18n::tr!("ik-edit-animation-add")))
                    .on_hover_text(aurora_i18n::tr!("ik-edit-animation-drop"))
                    .clicked()
                {
                    self.picker = !self.picker;
                    self.replacement = None;
                }
                if self.picker {
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(aurora_i18n::tr!("ik-edit-animation-search")));
                    if self.cache.as_ref() != Some(&(inv.generation, self.filter.clone())) {
                        let filter = self.filter.to_lowercase();
                        self.candidates = inv
                            .items
                            .keys()
                            .filter_map(|id| {
                                let (_, name) = item(inv, *id)?;
                                name.to_lowercase().contains(&filter).then_some((*id, name))
                            })
                            .collect();
                        self.candidates
                            .sort_by(|a, b| crate::natural_order::cmp(&a.1, &b.1).then(a.0.cmp(&b.0)));
                        self.cache = Some((inv.generation, self.filter.clone()));
                    }
                    let mut chosen = None;
                    egui::ScrollArea::vertical()
                        .id_salt("ik-animation-picker")
                        .max_height(160.0)
                        .show_rows(
                            ui,
                            ui.text_style_height(&egui::TextStyle::Body) + 6.0,
                            self.candidates.len(),
                            |ui, range| {
                                for i in range {
                                    let (id, name) = &self.candidates[i];
                                    if ui.selectable_label(false, name).clicked() {
                                        chosen = Some(*id);
                                    }
                                }
                            },
                        );
                    if let Some(id) = chosen {
                        self.add(inv, id);
                        self.picker = false;
                    }
                    ui.label(aurora_i18n::tr!("ik-edit-animation-inventory-hint"));
                }
            })
            .response;
        if let Some(payload) = response.dnd_release_payload::<crate::ui::inventory::InvDrag>() {
            self.add(inv, payload.0);
        }
    }
}

fn frame_controls(ui: &mut egui::Ui, frame: &mut u32, last: u32) {
    *frame = (*frame).min(last);
    ui.label(aurora_i18n::tr!("ik-edit-animation-frame"));
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(frame, 0..=last).show_value(false));
        ui.add(egui::DragValue::new(frame).range(0..=last).speed(1));
        ui.label(format!("/ {last}"));
    });
}

impl Editor {
    pub fn tick_animations(&mut self, streamer: &mut AnimStreamer, inv: &mut Inventory) {
        if self.open && !self.poser.active {
            if self.project.picking_notecard()
                && let Some(root) = actions::system(inv, 7)
            {
                actions::request_tree(inv, root);
            }
            self.animations.tick(streamer, inv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ik_edit::tests::HeadlessContext as _;
    use aurora_net::inventory::InvItem;
    #[test]
    fn dragging_a_layer_moves_its_frame_and_loaded_asset_together() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let mut stack = Stack::default();
        stack.demo(&lib.rig, "animations");
        let first = stack.rows[0].asset;
        let clip = stack.rows[0].clip.clone().unwrap();
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let palette = theme.palette();
        let inv = Inventory::default();
        let mut time = 0.0;
        let mut draw = |stack: &mut Stack, events| {
            time += 1.0 / 60.0;
            ctx.run_headless(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 700.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| stack.show(ui, &palette, &inv),
            )
        };
        let _ = draw(&mut stack, Vec::new());
        let output = draw(&mut stack, Vec::new());
        let positions: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "≡" => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .collect();
        assert_eq!(positions.len(), stack.rows.len(), "each layer exposes its actual drag handle");
        let start = positions[0];
        let end = positions[1];
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = draw(&mut stack, vec![egui::Event::PointerMoved(start), button(start, true)]);
        let _ = draw(&mut stack, vec![egui::Event::PointerMoved(end)]);
        let _ = draw(&mut stack, vec![button(end, false)]);
        assert_eq!(stack.rows[1].asset, first);
        assert_eq!(stack.rows[1].frame, 15);
        assert!(Arc::ptr_eq(stack.rows[1].clip.as_ref().unwrap(), &clip));
    }
    #[test]
    fn inventory_layers_resolve_links_keep_independent_frames_and_reorder_without_reloading() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let mut stack = Stack::default();
        stack.demo(&lib.rig, "animations");
        let asset = stack.rows[0].asset;
        let frame = stack.rows[0].frame;
        let clip = stack.rows[0].clip.clone().unwrap();
        stack.reorder(0, 1);
        assert_eq!(stack.rows[1].asset, asset);
        assert_eq!(stack.rows[1].frame, frame);
        assert!(Arc::ptr_eq(stack.rows[1].clip.as_ref().unwrap(), &clip));
        stack.rows[0].enabled = false;
        assert_eq!(stack.samples().len(), 1);
        assert_eq!(stack.samples()[0].frame, frame);
        let mut inv = Inventory::default();
        let original = Uuid::from_u128(1);
        let link = Uuid::from_u128(2);
        inv.items.insert(
            original,
            InvItem {
                id: original,
                asset_type: 20,
                asset_id: asset,
                name: "Test animation".into(),
                ..Default::default()
            },
        );
        inv.items.insert(
            link,
            InvItem {
                id: link,
                asset_type: 24,
                asset_id: original,
                ..Default::default()
            },
        );
        stack.add(&inv, link);
        assert_eq!(stack.rows.len(), 3);
        assert_eq!(stack.rows[2].asset, asset);
        assert_eq!(stack.samples().len(), 1, "pending assets contribute no pose");
        stack.add(&inv, Uuid::nil());
        assert_eq!(stack.rows.len(), 3);
        for _ in 0..30 {
            stack.add(&inv, original);
        }
        assert_eq!(stack.rows.len(), LIMIT);
    }
    fn animation(id: u128, asset: u128, owner_mask: u32) -> InvItem {
        InvItem {
            id: Uuid::from_u128(id),
            asset_type: 20,
            asset_id: Uuid::from_u128(asset),
            name: format!("Animation {id}"),
            owner_mask,
            ..Default::default()
        }
    }
    #[test]
    fn project_rows_name_the_asset_of_full_permission_items_only() {
        let mut inv = Inventory::default();
        let full = animation(1, 0xa1, actions::FULL);
        let no_copy = animation(2, 0xa2, actions::FULL & !actions::COPY);
        let no_transfer = animation(3, 0xa3, actions::FULL & !actions::TRANSFER);
        for it in [&full, &no_copy, &no_transfer] {
            inv.items.insert(it.id, it.clone());
        }
        let mut stack = Stack::default();
        for it in [&full, &no_copy, &no_transfer] {
            stack.add(&inv, it.id);
        }
        let mut saved = stack.save();
        assert!(
            saved.rows.iter().all(|r| r.asset.is_some()),
            "the in-memory profile copy keeps every asset"
        );
        saved.name_assets(|id| shared_asset(&inv, id));
        let assets: Vec<_> = saved.rows.iter().map(|r| (r.item, r.asset)).collect();
        assert_eq!(
            assets,
            [
                (Some(full.id), Some(full.asset_id)),
                (Some(no_copy.id), None),
                (Some(no_transfer.id), None),
            ]
        );
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains(&full.asset_id.to_string()));
        assert!(!json.contains(&no_copy.asset_id.to_string()));
        assert!(!json.contains(&no_transfer.asset_id.to_string()));
        assert!(json.contains(&no_copy.id.to_string()), "the row still names its item");
        let mut background = stack.save();
        background.name_assets(|id| SharedAssets::new(&inv).get(id));
        assert_eq!(background, saved);
        let mut stripped = saved.clone();
        stripped.rows[0].asset = None;
        let mut restored = Stack::default();
        restored.restore(&stripped);
        assert_eq!(restored.rows[1].asset, Uuid::nil());
        assert_eq!(
            restored.rows.iter().map(|r| r.item).collect::<Vec<_>>(),
            stack.rows.iter().map(|r| r.item).collect::<Vec<_>>()
        );
    }
    #[test]
    fn a_row_with_only_an_asset_id_stays_missing_while_inventory_rows_resolve_their_item() {
        let mut inv = Inventory::default();
        let owned = animation(1, 0xb1, actions::FULL & !actions::COPY);
        inv.items.insert(owned.id, owned.clone());
        let row = |item: Option<Uuid>, asset: u128| SavedRow {
            item,
            asset: Some(Uuid::from_u128(asset)),
            name: "Layer".into(),
            frame: 3,
            enabled: true,
        };
        let saved = SavedStack {
            rows: vec![
                // Another agent's file: an asset id with no item of ours.
                row(None, 0xb2),
                row(Some(Uuid::from_u128(9)), 0xb3),
                // Our item wins over the asset id written next to it.
                row(Some(owned.id), 0xb4),
            ],
            suppress_live: false,
        };
        assert!(saved.valid());
        // Even an asset already loaded in the viewer is not played by id.
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let mut demo = Stack::default();
        demo.demo(&lib.rig, "animations");
        let clip = demo.rows[0].clip.clone().unwrap();
        let dir = std::env::temp_dir().join(format!("aurora-ik-rows-{}", Uuid::new_v4()));
        let mut streamer = AnimStreamer::new(dir.clone());
        for asset in [0xb2, 0xb3, 0xb4] {
            streamer.insert(Uuid::from_u128(asset), clip.clone());
        }
        let mut stack = Stack::default();
        stack.restore(&saved);
        stack.tick(&mut streamer, &mut inv);
        assert!(stack.rows[0].failed && stack.rows[0].clip.is_none());
        assert!(stack.rows[1].failed && stack.rows[1].clip.is_none());
        assert_eq!(stack.rows[2].asset, owned.asset_id);
        assert_eq!(stack.rows[2].name, owned.name);
        assert!(stack.rows[2].clip.is_none(), "the item's own asset is still loading");
        assert!(!stack.rows[2].failed);
        assert!(stack.samples().is_empty());
        streamer.insert(owned.asset_id, clip.clone());
        stack.tick(&mut streamer, &mut inv);
        assert_eq!(stack.samples().len(), 1);
        assert!(stack.rows[0].failed && stack.rows[1].failed);
        let _ = std::fs::remove_dir_all(dir);
    }
}
