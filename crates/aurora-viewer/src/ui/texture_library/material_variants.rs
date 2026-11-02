//! The Variants card of the inventory material editor (#1372): the original
//! material and its variants, a selector opening one in place, and new,
//! duplicate, rename and delete. An Aurora addition: the Legacy Viewer has no
//! variants. A new variant goes through the Save As steps of
//! LLMaterialEditor::onSaveAsMsgCallback (indra/newview/llmaterialeditor.cpp):
//! copy the item next to the original, then save its asset when it differs;
//! then its description takes the tag naming its original.
use super::*;
use aurora_assets::PbrMaterial;

/// The inventory name limit applied by `rules::clean_name`.
const NAME_CHARS: usize = 63;

/// `base suffix`, the base shortened so the suffix always survives the
/// inventory name limit: two variants never collapse into one name.
pub(crate) fn variant_name(base: &str, suffix: &str) -> String {
    let suffix: String = suffix.trim().chars().take(NAME_CHARS).collect();
    let room = NAME_CHARS.saturating_sub(suffix.chars().count() + 1);
    let base: String = base.trim().chars().take(room).collect();
    let base = base.trim_end();
    if base.is_empty() { suffix } else { format!("{base} {suffix}") }
}

/// The card's state in one material editor.
#[derive(Default)]
pub(crate) struct Panel {
    /// The label being typed by Rename.
    pub renaming: Option<String>,
    /// The name of a duplicate asked for this frame: the editor saves its
    /// draft into it once this frame's edits are in.
    pub duplicate: Option<String>,
}

/// Inventory has no grouping of its own: a variant's description ends with
/// `[variant:<original item>]`, kept on the server, so every viewer and every
/// computer sees the same family whatever the names.
const TAG: &str = "[variant:";
/// The inventory description limit.
const DESC_CHARS: usize = 127;

/// The original a description's tag names.
pub(crate) fn tag(desc: &str) -> Option<Uuid> {
    let start = desc.rfind(TAG)? + TAG.len();
    let end = desc[start..].find(']')? + start;
    Uuid::parse_str(&desc[start..end]).ok()
}

/// `desc` with its tag naming `root` in place of any other, the user's text
/// shortened so the tag always fits.
fn tagged(desc: &str, root: Uuid) -> String {
    let mut text = desc.to_owned();
    while let Some(start) = text.rfind(TAG) {
        let end = text[start..].find(']').map_or(text.len(), |e| start + e + 1);
        text.replace_range(start..end, "");
    }
    let tag = format!("{TAG}{root}]");
    let room = DESC_CHARS.saturating_sub(tag.chars().count() + 1);
    let text: String = text.trim().chars().take(room).collect();
    let text = text.trim_end();
    if text.is_empty() { tag } else { format!("{text} {tag}") }
}

/// The patch giving `item` the tag of `root`, or none when it has it.
pub(crate) fn tag_change(inv: &crate::world::inventory::Inventory, item: Uuid, root: Uuid) -> Option<Mutation> {
    let it = inv.items.get(&item)?;
    (tag(&it.desc) != Some(root)).then(|| Mutation {
        operations: vec![Operation::Patch {
            id: item,
            folder: false,
            body: aurora_llsd::llsd_map! { "desc" => tagged(&it.desc, root) },
        }],
        refresh: vec![it.parent],
        ..Default::default()
    })
}

/// A material's family: its original and the materials whose tag names it,
/// wherever they are, out of the trash.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Family {
    /// The original's item, which every variant's tag names, even after it
    /// is deleted.
    pub root: Uuid,
    /// The original when present, then the variants by name.
    pub members: Vec<Member>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub id: Uuid,
    pub asset: Uuid,
    pub name: String,
}
impl Family {
    pub fn contains(&self, id: Uuid) -> bool {
        self.members.iter().any(|m| m.id == id)
    }
    pub fn has_original(&self) -> bool {
        self.members.first().is_some_and(|m| m.id == self.root)
    }
    /// `base word`, then `base word 2`, `base word 3`, whichever no member
    /// is called.
    pub fn free_name(&self, base: &str, word: &str) -> String {
        (1..)
            .map(|n| variant_name(base, &if n == 1 { word.to_owned() } else { format!("{word} {n}") }))
            .find(|name| !self.members.iter().any(|m| m.name.eq_ignore_ascii_case(name)))
            .unwrap_or_default()
    }
}

pub(crate) fn family(inv: &crate::world::inventory::Inventory, item: &aurora_net::inventory::InvItem) -> Family {
    let root = tag(&item.desc).unwrap_or(item.id);
    let mut members: Vec<Member> = inv
        .items
        .values()
        .filter(|it| it.asset_type == 57 && it.id != item.id && (it.id == root || tag(&it.desc) == Some(root)))
        .filter(|it| !rules::in_type(inv, it.id, 14))
        .chain(std::iter::once(item))
        .map(|it| Member {
            id: it.id,
            asset: it.asset_id,
            name: it.name.clone(),
        })
        .collect();
    members.sort_by(|a, b| {
        (b.id == root)
            .cmp(&(a.id == root))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    Family { root, members }
}

/// How a material looks until its small ball is built: its base colour tint
/// over its base colour texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Swatch {
    pub tint: [u8; 3],
    pub texture: Option<Uuid>,
}
impl Swatch {
    pub fn of(material: &PbrMaterial) -> Self {
        let [r, g, b, _] = material.base_color_factor;
        Self {
            tint: [r, g, b].map(egui::ecolor::gamma_u8_from_linear_f32),
            texture: material.base_color_texture,
        }
    }
}

fn paint_swatch(ui: &egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>, swatch: Option<Swatch>, rect: egui::Rect) {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let Some(swatch) = swatch else {
        ui.painter().rect_filled(rect, radius, p.field);
        return;
    };
    let [r, g, b] = swatch.tint;
    let tint = egui::Color32::from_rgb(r, g, b);
    match swatch.texture.and_then(|t| images.get(&t)) {
        Some(image) => egui::Image::new(image).tint(tint).corner_radius(radius).paint_at(ui, rect),
        None => {
            ui.painter().rect_filled(rect, radius, tint);
        }
    }
}

/// What the Variants card needs from its editor and window.
pub(crate) struct Card<'a> {
    pub item: &'a aurora_net::inventory::InvItem,
    pub world: &'a World,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub family: &'a Family,
    /// By item: the current one from the draft, the others as saved.
    pub swatches: &'a HashMap<Uuid, Swatch>,
    /// By item, once built: the same material ball as the library's cards,
    /// drawn instead of the flat swatch.
    pub balls: &'a HashMap<Uuid, std::sync::Arc<aurora_render::preview::Model>>,
    pub creation: Option<&'a Creation>,
    /// Unsaved edits or a save in flight: switching would lose or mix them.
    pub locked: bool,
}

/// What the pointer did in the Variants card this frame.
#[derive(Default)]
pub(crate) struct Picked {
    /// The variant to open in the editor's window.
    pub switch: Option<Uuid>,
    /// The variant under the pointer in the list, `(item, asset)`, for the
    /// preview to show until the pointer leaves it.
    pub preview: Option<(Uuid, Uuid)>,
}

fn one_line(ui: &egui::Ui, text: String, color: egui::Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, egui::TextStyle::Button.resolve(ui.style()), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// The selector's face: the open variant and a chevron, in the theme's
/// button look, opening the list below it.
fn selector(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let galley = one_line(ui, text.to_owned(), visuals.text_color(), rect.width() - 40.0);
    ui.painter().galley(
        egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
        galley,
        visuals.text_color(),
    );
    if let Some(chevron) = crate::ui::icons::global("chevron-down") {
        let at = egui::Rect::from_center_size(rect.right_center() - egui::vec2(14.0, 0.0), Vec2::splat(14.0));
        ui.painter().image(
            chevron.id(),
            at,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            visuals.text_color(),
        );
    }
    response
}

/// An icon button whose place the demo can find, by `key` and family.
fn keyed_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool, key: &str, root: Uuid) -> bool {
    let button = ui.scope(|ui| widgets::icon_button(ui, p, icon, tip, enabled));
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique((key.to_owned(), root)), button.response.rect));
    button.inner
}

/// The top of the Variants card: the selector and its actions.
pub(crate) fn card(panel: &mut Panel, ui: &mut egui::Ui, p: &Palette, card: Card, actions: &mut Vec<Action>) -> Picked {
    let Card {
        item,
        world,
        images,
        family,
        swatches,
        balls,
        creation,
        locked,
    } = card;
    // A member's ball when built, else its flat swatch.
    let paint = |ui: &egui::Ui, id: Uuid, rect: egui::Rect| match balls.get(&id) {
        Some(model) => aurora_render::preview::paint_in(
            ui,
            rect,
            egui::Id::unique(("variant_ball", id, rect.width() as u32)),
            model.clone(),
            previews::view(p),
        ),
        None => paint_swatch(ui, p, images, swatches.get(&id).copied(), rect),
    };
    let inv = &world.inventory;
    let root = family.root;
    let mut switch = None;
    let mut preview = None;
    let running = creation.is_some_and(Creation::pending_any);
    // Library and Marketplace copies go to the Materials folder, away from
    // the family (LLMaterialEditor::onSaveAsMsgCallback).
    let here = !rules::library(inv, item.id) && !rules::in_type(inv, item.id, 53);
    // A variant is copied, then tagged and edited: Copy and Modify.
    let variable = |id: Uuid| {
        inv.items
            .get(&id)
            .is_some_and(|it| it.owner_mask & (rules::COPY | rules::MODIFY) == rules::COPY | rules::MODIFY)
    };
    let current = family.members.iter().find(|m| m.id == item.id);
    let is_root = root == item.id;
    let label = |m: &Member| {
        if m.id == root {
            aurora_i18n::tr!("material-variants-original", name = m.name.as_str())
        } else {
            m.name.clone()
        }
    };
    // New: a copy of the original as saved; the copy already carries its
    // asset, so nothing more is saved. Alone, a material shows only this
    // in its header; the selector and the other actions come with its
    // first variant.
    let alone = family.members.len() <= 1;
    let can_new = here && family.has_original() && variable(root) && !running;
    let new = aurora_i18n::tr!("material-variants-add");
    let mut add = false;
    ui.horizontal(|ui| {
        widgets::section_header(ui, p, "stack-2-filled", aurora_i18n::tr!("material-variants-title"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if alone {
                add = keyed_button(ui, p, "plus", new, can_new, "resource_material_variants_add", root);
            } else {
                ui.label(
                    RichText::new(aurora_i18n::tr!("material-variants-count", count = family.members.len()))
                        .small()
                        .color(p.muted),
                );
            }
        });
    });
    if !alone {
        ui.horizontal(|ui| {
            let (at, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
            paint(ui, item.id, at);
            let buttons = 4.0 * (24.0 + ui.spacing().item_spacing.x);
            let width = (ui.available_width() - buttons).max(80.0);
            ui.add_enabled_ui(!locked, |ui| {
                let button = selector(ui, &current.map(label).unwrap_or_else(|| item.name.clone()), width);
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::unique(("resource_material_variants_select", root)), button.rect));
                // Narrower than the selector, like the mockup of #1372.
                let popup = (width * 0.55).max(width.min(200.0));
                egui::Popup::menu(&button).width(popup).show(|ui| {
                    ui.set_min_width(popup);
                    for (index, member) in family.members.iter().enumerate() {
                        let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), egui::Sense::click());
                        let open = member.id == item.id;
                        let visuals = ui.visuals();
                        if open || row.hovered() {
                            let fill = if open {
                                visuals.selection.bg_fill
                            } else {
                                visuals.widgets.hovered.weak_bg_fill
                            };
                            ui.painter().rect_filled(rect, visuals.widgets.inactive.corner_radius, fill);
                        }
                        let side = Vec2::splat(26.0);
                        let at = egui::Rect::from_min_size(rect.left_center() - egui::vec2(-4.0, side.y * 0.5), side);
                        paint(ui, member.id, at);
                        let text = one_line(ui, label(member), visuals.text_color(), rect.width() - side.x - 44.0);
                        ui.painter().galley(
                            egui::pos2(at.right() + 8.0, rect.center().y - text.size().y * 0.5),
                            text,
                            visuals.text_color(),
                        );
                        if open && let Some(check) = crate::ui::icons::global("check") {
                            let mark = egui::Rect::from_center_size(rect.right_center() - egui::vec2(14.0, 0.0), Vec2::splat(16.0));
                            ui.painter().image(
                                check.id(),
                                mark,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                p.ink,
                            );
                        }
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(egui::Id::unique(("resource_material_variant_row", index)), rect));
                        if row.hovered() {
                            preview = Some((member.id, member.asset));
                        }
                        if row.clicked() && !open {
                            switch = Some(member.id);
                            ui.close();
                        }
                    }
                });
            });
            add |= keyed_button(ui, p, "plus", new, can_new, "resource_material_variants_add", root);
            let duplicate = aurora_i18n::tr!("material-variants-duplicate");
            if keyed_button(
                ui,
                p,
                "copy-plus-filled",
                duplicate,
                here && variable(item.id) && !running,
                "resource_material_variants_duplicate",
                root,
            ) {
                panel.duplicate = Some(family.free_name(&item.name, aurora_i18n::tr!("common-copy")));
            }
            // Modify, as in LLItemBridge::isItemRenameable.
            let renamable = super::material_editor::can_edit(world, item);
            if widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"), renamable && !running) {
                panel.renaming = if panel.renaming.is_some() {
                    None
                } else {
                    current.map(|m| m.name.clone())
                };
            }
            let deletable = !is_root && !rules::library(inv, item.id) && !rules::in_type(inv, item.id, 14) && !running;
            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("material-variants-delete"), deletable) {
                switch = family.members.iter().map(|m| m.id).find(|id| *id != item.id);
                actions.push(Action::Discard(item.id));
            }
        });
    }
    if add && let Some(original) = family.members.first() {
        actions.push(Action::SaveMaterialVariant {
            item: root,
            root,
            name: family.free_name(&original.name, aurora_i18n::tr!("material-variants-new")),
            material: None,
        });
    }
    if locked && !alone {
        ui.label(RichText::new(aurora_i18n::tr!("material-variants-locked")).small().color(p.muted));
    }
    if !here {
        ui.label(
            RichText::new(aurora_i18n::tr!("material-variants-elsewhere"))
                .small()
                .color(p.muted),
        );
    }
    if let Some(name) = &mut panel.renaming {
        let mut done = false;
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 2.0 * (24.0 + ui.spacing().item_spacing.x)).max(60.0);
            let field = ui.add(egui::TextEdit::singleline(name).char_limit(NAME_CHARS).desired_width(width));
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let valid = rules::clean_name(name).is_ok();
            if (widgets::icon_button(ui, p, "check", aurora_i18n::tr!("common-rename"), valid) || entered && valid)
                && let Some(change) = rename(inv, item.id, name)
            {
                actions.push(Action::Change(change));
                done = true;
            }
            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), true) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
        if done {
            panel.renaming = None;
        }
    }
    match creation.filter(|c| family.contains(c.source)).map(|c| (&c.name, &c.outcome)) {
        Some((_, Outcome::Pending)) => {
            ui.label(RichText::new(aurora_i18n::tr!("resource-preview-saving")).small().color(p.muted));
        }
        Some((name, Outcome::Failed(reason))) => {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "material-variants-failed",
                    name = name.as_str(),
                    reason = reason.as_str()
                ))
                .small()
                .color(p.warn),
            );
        }
        _ => {}
    }
    Picked { switch, preview }
}

/// The patch renaming one material: names no longer tie a family.
fn rename(inv: &crate::world::inventory::Inventory, id: Uuid, typed: &str) -> Option<Mutation> {
    let name = rules::clean_name(typed).ok()?;
    let it = inv.items.get(&id).filter(|it| it.name != name)?;
    Some(Mutation {
        operations: vec![Operation::Patch {
            id,
            folder: false,
            body: aurora_llsd::llsd_map! { "name" => name },
        }],
        refresh: vec![it.parent],
        ..Default::default()
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Pending,
    /// The new item, for its family's window to open.
    Created(Uuid),
    Failed(String),
}

/// The variant being created, one at a time. It lives in the library, not in
/// the editor, so closing the editor does not stop it.
pub(crate) struct Creation {
    /// The item copied: the original for New, the open variant for Duplicate.
    pub source: Uuid,
    /// The original the copy's tag names.
    pub root: Uuid,
    /// The copy, once made: it is tagged before it counts as created.
    copy: Option<Uuid>,
    pub request: Uuid,
    pub name: String,
    /// The asset to save into the copy, or none when the copy is already it.
    material: Option<PbrMaterial>,
    pub outcome: Outcome,
}
impl Creation {
    pub fn new(source: Uuid, root: Uuid, name: String, material: Option<PbrMaterial>) -> Self {
        Self {
            source,
            root,
            copy: None,
            request: Uuid::new_v4(),
            name,
            material,
            outcome: Outcome::Pending,
        }
    }
    fn pending_any(&self) -> bool {
        self.outcome == Outcome::Pending
    }
    pub fn pending(&self, request: Uuid) -> bool {
        self.request == request && self.pending_any()
    }
    /// What to save into the copy just made: `Some(None)` when nothing.
    pub fn material(&self, request: Uuid) -> Option<Option<&PbrMaterial>> {
        self.pending(request).then_some(self.material.as_ref())
    }
    pub fn copied(&mut self, request: Uuid, item: Uuid) {
        if self.pending(request) {
            self.copy = Some(item);
        }
    }
    pub fn copy(&self, request: Uuid) -> Option<Uuid> {
        self.copy.filter(|_| self.pending(request))
    }
    /// Ends the creation; returns the material saved, for the scene.
    pub fn answer(&mut self, request: Uuid, result: Result<Uuid, String>) -> Option<PbrMaterial> {
        if !self.pending(request) {
            return None;
        }
        match result {
            Ok(item) => {
                self.outcome = Outcome::Created(item);
                self.material.take()
            }
            Err(reason) => {
                self.outcome = Outcome::Failed(reason);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_their_suffix_within_the_inventory_limit() {
        assert_eq!(variant_name("Coat", "Variant"), "Coat Variant");
        let long = "x".repeat(80);
        let name = variant_name(&long, "Variant 2");
        assert_eq!(name.chars().count(), NAME_CHARS);
        assert!(name.ends_with(" Variant 2"));
        assert_eq!(variant_name("  ", "Red"), "Red");
        assert!(rules::clean_name(&name).is_ok_and(|clean| clean == name));
    }

    fn material(n: u128, parent: u128, name: &str, desc: &str) -> aurora_net::inventory::InvItem {
        aurora_net::inventory::InvItem {
            id: Uuid::from_u128(n),
            asset_id: Uuid::from_u128(100 + n),
            parent: Uuid::from_u128(parent),
            asset_type: 57,
            name: name.into(),
            desc: desc.into(),
            ..Default::default()
        }
    }
    fn of(n: u128) -> String {
        format!("{TAG}{}]", Uuid::from_u128(n))
    }

    #[test]
    fn tags_replace_one_another_and_keep_the_users_text_within_the_limit() {
        let root = Uuid::from_u128(1);
        assert_eq!(tag(&tagged("", root)), Some(root));
        assert_eq!(tagged("", root), of(1));
        assert_eq!(tagged("Red coat", root), format!("Red coat {}", of(1)));
        let moved = tagged(&tagged("Red coat", Uuid::from_u128(2)), root);
        assert_eq!(moved, format!("Red coat {}", of(1)), "one tag, the latest");
        let long = tagged(&"x".repeat(200), root);
        assert_eq!(long.chars().count(), DESC_CHARS);
        assert_eq!(tag(&long), Some(root));
        assert_eq!(tag("no tag"), None);
        assert_eq!(tag("[variant:not a uuid]"), None);
    }

    #[test]
    fn a_family_is_the_original_and_the_materials_tagged_with_it_whatever_their_names() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let items = vec![
            material(1, 9, "Coat", "my coat"),
            material(2, 9, "Crimson", &of(1)),
            material(3, 8, "Another folder", &format!("blue {}", of(1))),
            material(4, 9, "Coat Red", ""),
            material(5, 9, "Tree", &of(4)),
            aurora_net::inventory::InvItem {
                asset_type: 0,
                ..material(6, 9, "A texture", &of(1))
            },
        ];
        world.inventory.add_items(items.clone());
        let names = |f: &Family| f.members.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
        // from any member, the same family, original first
        for open in [0, 1, 2] {
            let family = family(&world.inventory, &items[open]);
            assert_eq!(family.root, Uuid::from_u128(1));
            assert!(family.has_original());
            assert_eq!(names(&family), ["Coat", "Another folder", "Crimson"]);
        }
        // a name alone ties nothing
        assert_eq!(names(&family(&world.inventory, &items[3])), ["Coat Red", "Tree"]);
        // the original gone, its variants keep each other
        let orphans = family(&world.inventory, &material(7, 9, "Orphan", &of(42)));
        assert!(!orphans.has_original());
        assert_eq!(orphans.members.len(), 1);
    }

    #[test]
    fn new_names_take_the_first_free_number() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let items = vec![
            material(1, 9, "Coat", ""),
            material(2, 9, "Coat Variant", &of(1)),
            material(3, 9, "coat variant 2", &of(1)),
        ];
        world.inventory.add_items(items.clone());
        let family = family(&world.inventory, &items[0]);
        assert_eq!(family.free_name("Coat", "Variant"), "Coat Variant 3");
        assert_eq!(family.free_name("Coat Variant", "Copy"), "Coat Variant Copy");
    }

    #[test]
    fn renaming_or_tagging_patches_only_what_changes() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let items = vec![material(1, 9, "Coat", ""), material(2, 9, "Coat Variant", "notes")];
        world.inventory.add_items(items.clone());
        let body = |change: Mutation, key: &str| match &change.operations[..] {
            [Operation::Patch { id, body, .. }] => (*id, body.get(key).as_str().to_owned()),
            _ => panic!("one patch"),
        };
        let renamed = rename(&world.inventory, items[1].id, "Crimson").expect("rename");
        assert_eq!(body(renamed, "name"), (items[1].id, "Crimson".to_owned()));
        assert!(rename(&world.inventory, items[1].id, "Coat Variant").is_none(), "unchanged");
        assert!(rename(&world.inventory, items[1].id, " ").is_none(), "invalid");
        let tagging = tag_change(&world.inventory, items[1].id, items[0].id).expect("tag");
        assert_eq!(body(tagging, "desc"), (items[1].id, format!("notes {}", of(1))));
        world.inventory.add_items(vec![material(2, 9, "Coat Variant", &of(1))]);
        assert!(tag_change(&world.inventory, items[1].id, items[0].id).is_none(), "already tagged");
    }

    #[test]
    fn a_creation_saves_only_what_the_copy_lacks_and_answers_once() {
        let plain = Creation::new(Uuid::from_u128(1), Uuid::from_u128(1), "Coat Variant".into(), None);
        assert_eq!(plain.material(plain.request), Some(None), "a plain copy saves nothing");
        assert_eq!(plain.material(Uuid::from_u128(99)), None);
        let red = PbrMaterial {
            roughness_factor: 0.25,
            ..Default::default()
        };
        let mut edited = Creation::new(Uuid::from_u128(1), Uuid::from_u128(1), "Coat Copy".into(), Some(red.clone()));
        let request = edited.request;
        assert_eq!(edited.material(request), Some(Some(&red)));
        assert_eq!(edited.copy(request), None);
        edited.copied(request, Uuid::from_u128(5));
        assert_eq!(edited.copy(request), Some(Uuid::from_u128(5)));
        assert!(edited.answer(Uuid::from_u128(99), Ok(Uuid::from_u128(5))).is_none());
        assert_eq!(edited.answer(request, Ok(Uuid::from_u128(5))), Some(red));
        assert_eq!(edited.outcome, Outcome::Created(Uuid::from_u128(5)));
        assert!(!edited.pending(request));
        assert!(edited.answer(request, Err("late".into())).is_none(), "answered once");
        let mut refused = Creation::new(Uuid::from_u128(1), Uuid::from_u128(1), "Coat Copy".into(), None);
        let request = refused.request;
        refused.answer(request, Err("Refused".into()));
        assert_eq!(refused.outcome, Outcome::Failed("Refused".into()));
    }
}
