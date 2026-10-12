//! Aurora import windows: stable layout, short labels, scoped advanced
//! controls and a persistent footer showing the exact confirmed price.

use super::*;
use crate::{
    theme::Palette,
    ui::widgets::{self, Floater, flat_button},
};
use egui::{RichText, Vec2};

pub enum Action {
    Files,
    Start,
    Cancel,
    Preview(Uuid),
    Play(Uuid, bool),
    Reload(Uuid),
    Quote(Uuid),
    MeshFile(Uuid, usize),
    Open(Uuid),
    Local(Uuid),
    MapFile(Uuid, usize),
    MapInventory(Uuid, usize),
    Remove(Uuid),
    Resume,
}

pub fn error(error: Error) -> String {
    if let Error::PriceChanged(Some(cost)) = error {
        return aurora_i18n::tr!("import-new-price", cost = cost);
    }
    match error {
        Error::Invalid => aurora_i18n::tr!("import-invalid-request"),
        Error::Unavailable => aurora_i18n::tr!("import-service-unavailable"),
        Error::Rejected => aurora_i18n::tr!("import-rejected"),
        Error::Response => aurora_i18n::tr!("import-invalid-response"),
        Error::Unknown => aurora_i18n::tr!("import-unknown-outcome"),
        Error::Verification => aurora_i18n::tr!("import-unverified"),
        Error::PriceChanged(_) => aurora_i18n::tr!("import-price-changed"),
    }
    .into()
}

fn status(entry: &Entry) -> String {
    match &entry.status {
        Status::Ready => aurora_i18n::tr!("import-ready").into(),
        Status::Preparing => aurora_i18n::tr!("import-preparing").into(),
        Status::Uploading => aurora_i18n::tr!("import-uploading").into(),
        Status::Done(_) => aurora_i18n::tr!("import-done").into(),
        Status::Cancelled => aurora_i18n::tr!("import-cancelled").into(),
        Status::Error(error) => error.clone(),
    }
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    imports: &mut Imports,
    prefs: &mut Preferences,
    world: &crate::world::World,
    inventory_prefs: &crate::ui::inventory::InventoryPreferences,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut open = imports.open;
    let mut close_requested = false;
    let prices = if imports.demo_started {
        Prices {
            texture: Some(10),
            large_texture: Some(40),
            sound: Some(10),
            animation: Some(10),
        }
    } else {
        world.login.as_ref().map(|login| Prices::from_login(&login.raw)).unwrap_or_default()
    };
    let prices = prices.with_fallback(world.upload_cost);
    let busy = imports.busy() || imports.confirm;
    let old_permissions = prefs.clone();
    let window_id = egui::Id::new("asset_imports_v3");
    let window_height = ctx
        .data_mut(|d| d.get_persisted::<Vec2>(window_id.with("expanded_size")))
        .map_or(650.0, |s| s.y);
    let height = (window_height - 230.0).clamp(160.0, (ctx.content_rect().height() - 260.0).max(160.0));
    let mut floater = Floater::new(
        "asset_imports_v3",
        aurora_i18n::tr!("import-title"),
        egui::pos2(100.0, 80.0),
        Vec2::new(840.0, 650.0),
    );
    floater.min_size = Vec2::new(720.0, 500.0);
    floater.show(ctx, p, &mut open, |ui| {
        ui.horizontal(|ui| {
            for choice in [
                Choice::Texture,
                Choice::Sound,
                Choice::Animation,
                Choice::Material,
                Choice::Mesh,
                Choice::Bulk,
                Choice::Local,
            ] {
                if ui
                    .add_enabled(!busy, egui::Button::new(choice.label()).selected(imports.choice == choice))
                    .clicked()
                {
                    imports.open(choice);
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && imports.pending_files.is_none(),
                    egui::Button::new(aurora_i18n::tr!("import-choose-files")),
                )
                .clicked()
            {
                actions.push(Action::Files);
            }
            if imports.pending_files.is_some() {
                ui.spinner();
                ui.label(aurora_i18n::tr!("import-loading"));
            }
            if let Some(id) = imports.active {
                if ui
                    .add_enabled(!busy, egui::Button::new(aurora_i18n::tr!("import-reload")))
                    .clicked()
                {
                    actions.push(Action::Reload(id));
                }
                if ui
                    .add_enabled(!busy, egui::Button::new(aurora_i18n::tr!("common-remove")))
                    .clicked()
                {
                    actions.push(Action::Remove(id));
                }
            }
            if imports.entries.iter().any(|e| matches!(e.status, Status::Cancelled))
                && ui
                    .add_enabled(!busy, egui::Button::new(aurora_i18n::tr!("import-resume")))
                    .clicked()
            {
                actions.push(Action::Resume);
            }
            if imports.choice == Choice::Bulk {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.checkbox(&mut imports.scale_bulk_1k, aurora_i18n::tr!("import-limit-bulk-1k"));
                });
            }
        });
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(225.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(225.0);
                widgets::section_header(ui, p, "files", aurora_i18n::tr!("import-files"));
                egui::ScrollArea::vertical()
                    .id_salt("import_files")
                    .max_height(height - 30.0)
                    .show(ui, |ui| {
                        for entry in imports.entries.iter_mut().filter(|e| imports.choice.accepts(e)) {
                            ui.horizontal(|ui| {
                                ui.add_enabled(
                                    !busy && matches!(entry.status, Status::Ready | Status::Error(_)),
                                    egui::Checkbox::without_text(&mut entry.selected),
                                );
                                if ui
                                    .add(egui::Button::selectable(imports.active == Some(entry.id), &entry.name).truncate())
                                    .on_hover_text(entry.path.to_string_lossy())
                                    .clicked()
                                {
                                    imports.active = Some(entry.id);
                                }
                            });
                            let color = if matches!(entry.status, Status::Error(_)) {
                                p.danger
                            } else if matches!(entry.status, Status::Done(_)) {
                                p.success
                            } else {
                                p.muted
                            };
                            ui.label(RichText::new(status(entry)).small().color(color));
                            ui.add_space(6.0);
                        }
                    });
            });
            let (separator, _) = ui.allocate_exact_size(Vec2::new(1.0, height), egui::Sense::hover());
            ui.painter()
                .vline(separator.center().x, separator.y_range(), egui::Stroke::new(1.0, p.raised));
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width()).max(260.0), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("import_details")
                        .max_height(height)
                        .show(ui, |ui| {
                            if let Some(entry) = imports.active.and_then(|id| imports.entries.iter_mut().find(|e| e.id == id)) {
                                entry.view.edge_color = egui::Rgba::from(p.teal).to_array();
                                entry.view.ambient = [0.2; 3];
                                let metadata = (entry.name.clone(), entry.description.clone(), entry.folder);
                                if entry.local {
                                    ui.checkbox(&mut entry.local_live, aurora_i18n::tr!("import-local-live"));
                                }
                                if let Status::Done(item) = entry.status
                                    && flat_button(ui, p, aurora_i18n::tr!("import-open-inventory")).clicked()
                                {
                                    actions.push(Action::Open(item));
                                }
                                ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready), |ui| {
                                    egui::Grid::new("import_metadata")
                                        .num_columns(2)
                                        .spacing(Vec2::new(12.0, 8.0))
                                        .show(ui, |ui| {
                                            ui.label(aurora_i18n::tr!("common-name"));
                                            ui.add(egui::TextEdit::singleline(&mut entry.name).desired_width(360.0).char_limit(63));
                                            ui.end_row();
                                            ui.label(aurora_i18n::tr!("common-description"));
                                            ui.add(
                                                egui::TextEdit::singleline(&mut entry.description)
                                                    .desired_width(360.0)
                                                    .char_limit(127),
                                            );
                                            ui.end_row();
                                            ui.label(aurora_i18n::tr!("import-destination"));
                                            let current = world
                                                .inventory
                                                .folders
                                                .get(&entry.folder)
                                                .map(|f| f.info.name.as_str())
                                                .unwrap_or(aurora_i18n::tr!("import-no-folder"));
                                            egui::ComboBox::from_id_salt("import_folder")
                                                .selected_text(current)
                                                .width(360.0)
                                                .show_ui(ui, |ui| {
                                                    let mut folders: Vec<_> = world
                                                        .inventory
                                                        .folders
                                                        .values()
                                                        .filter(|folder| {
                                                            crate::world::inventory::actions::destination(
                                                                &world.inventory,
                                                                folder.info.id,
                                                                &inventory_prefs.protected,
                                                            )
                                                        })
                                                        .collect();
                                                    folders.sort_by(|a, b| a.info.name.cmp(&b.info.name));
                                                    for folder in folders {
                                                        ui.selectable_value(&mut entry.folder, folder.info.id, &folder.info.name);
                                                    }
                                                });
                                            ui.end_row();
                                        });
                                    ui.add_space(12.0);
                                    if let Some(source) = &mut entry.source {
                                        match source {
                                            Source::Texture(image) => texture(
                                                ui,
                                                p,
                                                entry.id,
                                                image,
                                                entry.empty_alpha,
                                                &mut entry.options,
                                                &entry.preview,
                                                &mut actions,
                                                &mut entry.preview_generation,
                                                &mut entry.texture_target,
                                                &entry.model,
                                                &mut entry.view,
                                            ),
                                            Source::Sound(sound) => {
                                                widgets::section_header(ui, p, "speaker-high", aurora_i18n::tr!("import-sound"));
                                                ui.label(aurora_i18n::tr!(
                                                    "import-sound-info",
                                                    duration = format!("{:.2}", sound.duration()),
                                                    channels = sound.source_channels,
                                                    bits = sound.source_bits
                                                ));
                                                ui.label(RichText::new(aurora_i18n::tr!("import-sound-conversion")).color(p.muted));
                                                if flat_button(ui, p, aurora_i18n::tr!("common-play")).clicked() {
                                                    actions.push(Action::Play(entry.id, true));
                                                }
                                                if flat_button(ui, p, aurora_i18n::tr!("common-stop")).clicked() {
                                                    actions.push(Action::Play(entry.id, false));
                                                }
                                            }
                                            Source::Animation { animation, bvh, .. } => {
                                                let before = (
                                                    animation.base_priority,
                                                    animation.looping,
                                                    animation.loop_in,
                                                    animation.loop_out,
                                                    animation.ease_in,
                                                    animation.ease_out,
                                                    animation.hand_pose,
                                                    animation.emote_name.clone(),
                                                );
                                                if let Some(animation) = Arc::get_mut(animation) {
                                                    animation_controls(ui, p, animation, bvh.as_ref());
                                                } else {
                                                    ui.label(aurora_i18n::tr!(
                                                        "import-animation-info",
                                                        duration = format!("{:.3}", animation.duration),
                                                        joints = animation.joints.len()
                                                    ));
                                                }
                                                let changed = before
                                                    != (
                                                        animation.base_priority,
                                                        animation.looping,
                                                        animation.loop_in,
                                                        animation.loop_out,
                                                        animation.ease_in,
                                                        animation.ease_out,
                                                        animation.hand_pose,
                                                        animation.emote_name.clone(),
                                                    );
                                                if changed && entry.player.bound.is_some() {
                                                    actions.push(Action::Play(entry.id, true));
                                                }
                                                ui.horizontal(|ui| {
                                                    if flat_button(ui, p, aurora_i18n::tr!("common-play")).clicked() {
                                                        actions.push(Action::Play(entry.id, true));
                                                    }
                                                    if flat_button(ui, p, aurora_i18n::tr!("common-stop")).clicked() {
                                                        entry.player.playing = false;
                                                        entry.player.seek(0.0);
                                                        actions.push(Action::Play(entry.id, false));
                                                    }
                                                    if flat_button(ui, p, aurora_i18n::tr!("import-pause")).clicked() {
                                                        entry.player.playing = !entry.player.playing;
                                                    }
                                                    if flat_button(ui, p, aurora_i18n::tr!("import-reload")).clicked() {
                                                        actions.push(Action::Reload(entry.id));
                                                    }
                                                });
                                                let mut position = animation.local_time(f64::from(entry.player.position)) as f32;
                                                if ui
                                                    .add(egui::Slider::new(&mut position, 0.0..=animation.duration).suffix(" s"))
                                                    .changed()
                                                {
                                                    entry.player.seek(position);
                                                }
                                                let old_base = entry.player.base;
                                                ui.checkbox(&mut entry.preview_on_avatar, aurora_i18n::tr!("import-preview-own-avatar"));
                                                let bases = [
                                                    aurora_i18n::tr!("import-base-standing"),
                                                    aurora_i18n::tr!("import-base-walking"),
                                                    aurora_i18n::tr!("import-base-sitting"),
                                                    aurora_i18n::tr!("import-base-flying"),
                                                ];
                                                egui::ComboBox::from_id_salt("animation_base")
                                                    .selected_text(bases[entry.player.base.min(3)])
                                                    .show_ui(ui, |ui| {
                                                        for (i, label) in bases.into_iter().enumerate() {
                                                            ui.selectable_value(&mut entry.player.base, i, label);
                                                        }
                                                    });
                                                if old_base != entry.player.base {
                                                    entry.player.seek(position);
                                                }
                                                if let Some(model) = &entry.model {
                                                    entry.view.background = [
                                                        p.field.r() as f32 / 255.0,
                                                        p.field.g() as f32 / 255.0,
                                                        p.field.b() as f32 / 255.0,
                                                        1.0,
                                                    ];
                                                    aurora_render::preview::paint_with_pose(
                                                        ui,
                                                        egui::Id::new(("animation_preview", entry.id)),
                                                        model.clone(),
                                                        &mut entry.view,
                                                        Vec2::new(ui.available_width(), 260.0),
                                                        entry.player.pose.clone(),
                                                    );
                                                }
                                            }
                                            Source::Material { value, images } => {
                                                if let Some(model) = &entry.model {
                                                    entry.view.material = Some(super::preview::material(value));
                                                    entry.view.background = [
                                                        p.field.r() as f32 / 255.0,
                                                        p.field.g() as f32 / 255.0,
                                                        p.field.b() as f32 / 255.0,
                                                        1.0,
                                                    ];
                                                    aurora_render::preview::paint(
                                                        ui,
                                                        egui::Id::new(("material_preview", entry.id)),
                                                        model.clone(),
                                                        &mut entry.view,
                                                        Vec2::new(ui.available_width(), 240.0),
                                                    );
                                                }
                                                let old = value.clone();
                                                let old_maps = images.each_ref().map(|image| image.as_ref().map(|i| i.key));
                                                material(ui, p, entry.id, value, images, &mut actions);
                                                if old != *value || old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key))
                                                {
                                                    entry.preview_generation += 1;
                                                }
                                                if old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key)) {
                                                    entry.model = None;
                                                    entry.preview_error = None;
                                                }
                                            }
                                            Source::Mesh(draft) => {
                                                if mesh_controls(ui, p, entry.id, draft, &mut actions) {
                                                    draft.invalidate();
                                                    entry.preview_generation += 1;
                                                    entry.model = None;
                                                }
                                                if let Some(model) = &entry.model {
                                                    entry.view.background = [
                                                        p.field.r() as f32 / 255.0,
                                                        p.field.g() as f32 / 255.0,
                                                        p.field.b() as f32 / 255.0,
                                                        1.0,
                                                    ];
                                                    aurora_render::preview::paint(
                                                        ui,
                                                        egui::Id::new(("mesh_preview", entry.id)),
                                                        model.clone(),
                                                        &mut entry.view,
                                                        Vec2::new(ui.available_width(), 260.0),
                                                    );
                                                }
                                                let old = (
                                                    draft.preview_lod,
                                                    draft.preview_physics,
                                                    draft.show_textures,
                                                    draft.preview_weights,
                                                    draft.show_joints,
                                                    draft.show_overrides,
                                                    draft.explode,
                                                );
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.label(aurora_i18n::tr!("import-preview"));
                                                    egui::ComboBox::from_id_salt("mesh_preview_lod")
                                                        .selected_text(lod_label(draft.preview_lod))
                                                        .show_ui(ui, |ui| {
                                                            for level in (0..4).rev() {
                                                                ui.selectable_value(&mut draft.preview_lod, level, lod_label(level));
                                                            }
                                                        });
                                                    ui.checkbox(&mut entry.view.edges, aurora_i18n::tr!("import-edges"));
                                                    ui.checkbox(&mut entry.view.uv, aurora_i18n::tr!("import-uv-guide"));
                                                    ui.checkbox(&mut draft.show_textures, aurora_i18n::tr!("import-textures"));
                                                    ui.checkbox(&mut draft.preview_physics, aurora_i18n::tr!("import-physics"));
                                                    ui.checkbox(&mut draft.preview_weights, aurora_i18n::tr!("import-preview-weights"));
                                                    ui.checkbox(&mut draft.show_joints, aurora_i18n::tr!("import-preview-joints"));
                                                    ui.checkbox(&mut draft.show_overrides, aurora_i18n::tr!("import-preview-overrides"));
                                                });
                                                if draft.preview_physics {
                                                    ui.add(
                                                        egui::Slider::new(&mut draft.explode, 0.0..=3.0)
                                                            .text(aurora_i18n::tr!("import-explode-hulls")),
                                                    );
                                                }
                                                if old
                                                    != (
                                                        draft.preview_lod,
                                                        draft.preview_physics,
                                                        draft.show_textures,
                                                        draft.preview_weights,
                                                        draft.show_joints,
                                                        draft.show_overrides,
                                                        draft.explode,
                                                    )
                                                {
                                                    entry.model = None;
                                                    entry.preview_generation += 1;
                                                }
                                            }
                                        }
                                    }
                                });
                                if metadata != (entry.name.clone(), entry.description.clone(), entry.folder) {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.invalidate();
                                    }
                                    entry.preview_generation += 1;
                                }
                                if matches!(entry.source, Some(Source::Mesh(_))) {
                                    ui.collapsing(aurora_i18n::tr!("import-load-settings"), |ui| {
                                        ui.checkbox(&mut prefs.mesh_auto_weights, aurora_i18n::tr!("import-auto-weights"));
                                        ui.checkbox(
                                            &mut prefs.mesh_auto_preview_weights,
                                            aurora_i18n::tr!("import-auto-preview-weights"),
                                        );
                                        ui.checkbox(&mut prefs.mesh_reliable_default, aurora_i18n::tr!("import-default-reliable"));
                                        ui.horizontal_wrapped(|ui| {
                                            for (label, values) in [
                                                (aurora_i18n::tr!("import-suffix-sl"), ["LOD0", "LOD1", "LOD2", "", "PHYS"]),
                                                (aurora_i18n::tr!("import-suffix-games"), ["LOD3", "LOD2", "LOD1", "LOD0", "PHYS"]),
                                                (aurora_i18n::tr!("import-suffix-words"), ["lowest", "low", "med", "high", "phys"]),
                                            ] {
                                                if flat_button(ui, p, label).clicked() {
                                                    prefs.mesh_suffixes = values.map(String::from);
                                                }
                                            }
                                        });
                                        for (level, suffix) in prefs.mesh_suffixes.iter_mut().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.label(if level < 4 {
                                                    lod_label(level)
                                                } else {
                                                    aurora_i18n::tr!("import-physics")
                                                });
                                                ui.add(egui::TextEdit::singleline(suffix).char_limit(32).desired_width(120.0));
                                            });
                                        }
                                    });
                                }
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(70.0);
                                    ui.label(RichText::new(aurora_i18n::tr!("import-empty")).color(p.muted));
                                });
                            }
                        });
                },
            );
        });
        ui.separator();
        ui.add_enabled_ui(!busy, |ui| {
            ui.collapsing(aurora_i18n::tr!("import-permissions"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("inventory-permissions-next-owner"));
                    ui.checkbox(&mut prefs.next_modify, aurora_i18n::tr!("inventory-permissions-modify"));
                    ui.checkbox(&mut prefs.next_copy, aurora_i18n::tr!("inventory-permissions-copy"));
                    if !prefs.next_copy {
                        prefs.next_transfer = true;
                    }
                    ui.add_enabled(
                        prefs.next_copy,
                        egui::Checkbox::new(&mut prefs.next_transfer, aurora_i18n::tr!("inventory-permissions-transfer")),
                    );
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut prefs.share_group, aurora_i18n::tr!("import-share-group"));
                    ui.checkbox(&mut prefs.everyone_copy, aurora_i18n::tr!("import-everyone-copy"));
                    ui.checkbox(&mut prefs.show_payment, aurora_i18n::tr!("import-show-payment"));
                });
            });
        });
        if !imports.message.is_empty() {
            ui.label(RichText::new(&imports.message).color(p.danger));
        }
        if prefs.show_payment && imports.spent > 0 {
            ui.label(RichText::new(aurora_i18n::tr!("import-paid", cost = imports.spent)).color(p.success));
        }
        ui.horizontal(|ui| {
            let total = imports.total(&prices);
            let local = imports.choice == Choice::Local;
            ui.label(
                RichText::new(if local {
                    aurora_i18n::tr!("import-local-free").into()
                } else {
                    match total {
                        Some(cost) => aurora_i18n::tr!("import-total", cost = cost),
                        None => aurora_i18n::tr!("import-price-unavailable").into(),
                    }
                })
                .strong(),
            );
            let selected = imports.entries.iter().any(|e| e.selected && matches!(e.status, Status::Ready));
            let can_upload = !busy && selected && total.is_some_and(|cost| world.balance.is_some_and(|b| b >= cost));
            if local {
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && ui
                        .add_enabled(
                            !entry.local_pending && entry.source.is_some(),
                            egui::Button::new(aurora_i18n::tr!("import-local-apply")),
                        )
                        .clicked()
                {
                    actions.push(Action::Local(entry.id));
                }
            } else if ui
                .add_enabled(can_upload, egui::Button::new(aurora_i18n::tr!("import-upload")))
                .clicked()
            {
                imports.confirm = true;
            }
            if busy {
                ui.spinner();
                if flat_button(ui, p, aurora_i18n::tr!("import-cancel-remaining")).clicked() {
                    actions.push(Action::Cancel);
                }
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-close")).clicked() {
                close_requested = true;
            }
        });
    });
    imports.open = open && !close_requested;
    if *prefs != old_permissions {
        for entry in &mut imports.entries {
            if let Some(Source::Mesh(draft)) = &mut entry.source {
                draft.invalidate();
                entry.preview_generation += 1;
            }
        }
    }
    if imports.confirm {
        let mut confirm = true;
        Floater::new(
            "import_confirmation",
            aurora_i18n::tr!("import-confirm-title"),
            egui::pos2(270.0, 240.0),
            Vec2::new(380.0, 160.0),
        )
        .fixed()
        .show(ctx, p, &mut confirm, |ui| {
            if let Some(cost) = imports.total(&prices) {
                ui.label(aurora_i18n::tr!("import-confirm-price", cost = cost));
            }
            ui.horizontal(|ui| {
                if flat_button(ui, p, aurora_i18n::tr!("import-upload")).clicked() {
                    actions.push(Action::Start);
                    imports.confirm = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    imports.confirm = false;
                }
            });
        });
        if !confirm {
            imports.confirm = false;
        }
    }
    actions
}

fn lod_label(level: usize) -> &'static str {
    match level {
        3 => aurora_i18n::tr!("import-lod-high"),
        2 => aurora_i18n::tr!("import-lod-medium"),
        1 => aurora_i18n::tr!("import-lod-low"),
        _ => aurora_i18n::tr!("import-lod-lowest"),
    }
}
fn lod_source(source: mesh::LodSource) -> &'static str {
    match source {
        mesh::LodSource::File => aurora_i18n::tr!("import-source-file"),
        mesh::LodSource::Auto => aurora_i18n::tr!("import-lod-auto"),
        mesh::LodSource::Combine => aurora_i18n::tr!("import-lod-combine"),
        mesh::LodSource::Fast => aurora_i18n::tr!("import-lod-fast"),
        mesh::LodSource::Reliable => aurora_i18n::tr!("import-lod-reliable"),
        mesh::LodSource::Above => aurora_i18n::tr!("import-lod-above"),
    }
}
fn mesh_controls(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft, actions: &mut Vec<Action>) -> bool {
    let old = draft.options.clone();
    let mut changed = false;
    widgets::section_header(ui, p, "cube", aurora_i18n::tr!("import-mesh"));
    let triangles = draft.scene.models.iter().map(|m| m.triangles()).sum::<usize>();
    ui.label(aurora_i18n::tr!(
        "import-mesh-info",
        models = draft.scene.models.len(),
        instances = draft.scene.instances.len(),
        triangles = triangles
    ));
    if !draft.scene.warnings.is_empty() {
        ui.collapsing(aurora_i18n::tr!("import-log"), |ui| {
            for warning in &draft.scene.warnings {
                ui.label(RichText::new(warning).color(p.warn));
            }
        });
    }
    ui.collapsing(aurora_i18n::tr!("import-lods"), |ui| {
        for level in (0..4).rev() {
            ui.push_id(level, |ui| {
                let lod = &mut draft.lods[level];
                ui.horizontal_wrapped(|ui| {
                    ui.label(lod_label(level));
                    let before = lod.source;
                    egui::ComboBox::from_id_salt("source")
                        .selected_text(lod_source(lod.source))
                        .show_ui(ui, |ui| {
                            for source in [
                                mesh::LodSource::File,
                                mesh::LodSource::Auto,
                                mesh::LodSource::Combine,
                                mesh::LodSource::Fast,
                                mesh::LodSource::Reliable,
                                mesh::LodSource::Above,
                            ] {
                                if level < 3 || source != mesh::LodSource::Above {
                                    ui.selectable_value(&mut lod.source, source, lod_source(source));
                                }
                            }
                        });
                    changed |= before != lod.source;
                    if lod.source == mesh::LodSource::File {
                        if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                            actions.push(Action::MeshFile(id, level));
                        }
                        if let Some(path) = &lod.path {
                            ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        }
                    } else if lod.source != mesh::LodSource::Above {
                        changed |= ui
                            .checkbox(&mut lod.error_mode, aurora_i18n::tr!("import-error-threshold"))
                            .changed();
                        changed |= if lod.error_mode {
                            ui.add(egui::DragValue::new(&mut lod.error).range(0.0..=1.0).speed(0.001)).changed()
                        } else {
                            ui.add(
                                egui::DragValue::new(&mut lod.triangles)
                                    .range(0..=triangles)
                                    .prefix(aurora_i18n::tr!("import-triangles")),
                            )
                            .changed()
                        };
                    }
                });
            });
        }
        ui.checkbox(&mut draft.options.normals, aurora_i18n::tr!("import-generate-normals"));
        ui.add_enabled(
            draft.options.normals,
            egui::Slider::new(&mut draft.options.crease, 0.0..=180.0).text(aurora_i18n::tr!("import-crease-angle")),
        );
    });
    ui.collapsing(aurora_i18n::tr!("import-physics"), |ui| {
        let labels = [
            aurora_i18n::tr!("import-convex-only"),
            lod_label(0),
            lod_label(1),
            lod_label(2),
            lod_label(3),
            aurora_i18n::tr!("import-bounding-box"),
            aurora_i18n::tr!("import-source-file"),
        ];
        egui::ComboBox::from_id_salt("physics_source")
            .selected_text(labels[(draft.options.physics_source + 1).clamp(0, 6) as usize])
            .show_ui(ui, |ui| {
                for (i, label) in labels.into_iter().enumerate() {
                    ui.selectable_value(&mut draft.options.physics_source, i as i32 - 1, label);
                }
            });
        if draft.options.physics_source == 5 && flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
            actions.push(Action::MeshFile(id, 4));
        }
        ui.add_enabled_ui(draft.options.physics_source >= 0, |ui| {
            ui.checkbox(&mut draft.options.decompose, aurora_i18n::tr!("import-decompose"));
            ui.add(egui::Slider::new(&mut draft.options.retain, 0.01..=1.0).text(aurora_i18n::tr!("import-retain")));
            ui.add_enabled_ui(draft.options.decompose, |ui| {
                ui.checkbox(&mut draft.options.surface, aurora_i18n::tr!("import-fill-surface"));
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("import-voxel-resolution"));
                    ui.add(egui::DragValue::new(&mut draft.options.resolution).range(1000..=16000000));
                });
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("import-max-hulls"));
                    ui.add(egui::DragValue::new(&mut draft.options.max_hulls).range(1..=256));
                });
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("import-hull-vertices"));
                    ui.add(egui::DragValue::new(&mut draft.options.max_vertices).range(3..=256));
                });
                ui.add(
                    egui::Slider::new(&mut draft.options.tolerance, 0.0001..=99.0)
                        .logarithmic(true)
                        .text(aurora_i18n::tr!("import-error-tolerance")),
                );
            });
        });
    });
    ui.collapsing(aurora_i18n::tr!("import-modifiers"), |ui| {
        ui.add(
            egui::Slider::new(&mut draft.options.scale, 0.01..=64.0)
                .logarithmic(true)
                .text(aurora_i18n::tr!("import-scale")),
        );
        ui.checkbox(&mut draft.options.textures, aurora_i18n::tr!("import-include-textures"));
    });
    ui.collapsing(aurora_i18n::tr!("import-rigging"), |ui| {
        let has_skin = draft.scene.models.iter().any(|m| m.skin.is_some());
        ui.add_enabled_ui(has_skin, |ui| {
            ui.checkbox(&mut draft.options.skin, aurora_i18n::tr!("import-include-weights"));
            ui.add_enabled_ui(draft.options.skin, |ui| {
                ui.checkbox(&mut draft.options.joints, aurora_i18n::tr!("import-include-joints"));
                ui.add_enabled_ui(draft.options.joints, |ui| {
                    ui.checkbox(&mut draft.options.lock_scale, aurora_i18n::tr!("import-lock-scale"));
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("import-pelvis-offset"));
                        ui.add(egui::DragValue::new(&mut draft.options.pelvis).range(-10.0..=10.0).speed(0.01));
                    });
                });
            });
            for skin in draft.scene.models.iter().filter_map(|m| m.skin.as_ref()) {
                ui.label(skin.names.join(", "));
            }
        });
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(draft.pending.is_none(), egui::Button::new(aurora_i18n::tr!("import-calculate-fee")))
            .clicked()
        {
            actions.push(Action::Quote(id));
        }
        if draft.pending.is_some() {
            ui.spinner();
            if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                draft.invalidate();
            }
        }
    });
    if let Some(quote) = &draft.quote {
        ui.label(aurora_i18n::tr!("import-total", cost = quote.cost));
        for (key, label) in [
            ("resource_cost", aurora_i18n::tr!("import-land-impact")),
            ("model_streaming_cost", aurora_i18n::tr!("import-streaming-cost")),
            ("simulation_cost", aurora_i18n::tr!("import-server-cost")),
            ("physics_cost", aurora_i18n::tr!("import-physics-cost")),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.label(format!("{:.3}", quote.data[key].as_f64()));
            });
        }
    }
    changed || old != draft.options
}

fn texture(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    image: &DecodedImage,
    empty_alpha: bool,
    options: &mut import_texture::Options,
    preview: &Option<egui::TextureHandle>,
    actions: &mut Vec<Action>,
    generation: &mut u64,
    target: &mut usize,
    model: &Option<Arc<aurora_render::preview::Model>>,
    view: &mut aurora_render::preview::View,
) {
    widgets::section_header(ui, p, "image", aurora_i18n::tr!("import-preview"));
    let old = *options;
    let old_target = *target;
    let labels = [
        aurora_i18n::tr!("import-preview-image"),
        aurora_i18n::tr!("import-preview-hair"),
        aurora_i18n::tr!("import-preview-female-head"),
        aurora_i18n::tr!("import-preview-female-upper"),
        aurora_i18n::tr!("import-preview-female-lower"),
        aurora_i18n::tr!("import-preview-male-head"),
        aurora_i18n::tr!("import-preview-male-upper"),
        aurora_i18n::tr!("import-preview-male-lower"),
        aurora_i18n::tr!("import-preview-skirt"),
        aurora_i18n::tr!("import-preview-sculpt"),
    ];
    egui::ComboBox::from_id_salt("import_texture_target")
        .selected_text(labels[(*target).min(9)])
        .show_ui(ui, |ui| {
            for (index, label) in labels.into_iter().enumerate() {
                ui.selectable_value(target, index, label);
            }
        });
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-max-size"));
        for (axis, value) in [("X", &mut options.max_width), ("Y", &mut options.max_height)] {
            egui::ComboBox::from_id_salt(("import_dimension", axis))
                .selected_text(value.to_string())
                .show_ui(ui, |ui| {
                    for size in [4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048] {
                        ui.selectable_value(value, size, size.to_string());
                    }
                });
        }
    });
    let (width, height) = import_texture::dimensions(image.width, image.height, *options);
    ui.label(aurora_i18n::tr!(
        "import-texture-size",
        source_width = image.width,
        source_height = image.height,
        width = width,
        height = height
    ));
    if width * height <= 128 * 128 {
        ui.checkbox(&mut options.lossless_small, aurora_i18n::tr!("import-lossless"));
    }
    if empty_alpha {
        ui.checkbox(&mut options.strip_alpha, aurora_i18n::tr!("import-strip-alpha"));
    }
    if old != *options || old_target != *target {
        *generation += 1;
        actions.push(Action::Preview(id));
    }
    if *target != 0 {
        if let Some(model) = model {
            view.background = [
                p.field.r() as f32 / 255.0,
                p.field.g() as f32 / 255.0,
                p.field.b() as f32 / 255.0,
                1.0,
            ];
            aurora_render::preview::paint(
                ui,
                egui::Id::new(("texture_preview", id)),
                model.clone(),
                view,
                Vec2::new(ui.available_width(), 260.0),
            );
        } else {
            ui.spinner();
        }
    } else if let Some(texture) = preview {
        let size = texture.size_vec2();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 260.0), egui::Sense::drag());
        let painter = ui.painter().with_clip_rect(rect);
        for y in 0..(rect.height() / 16.0).ceil() as i32 {
            for x in 0..(rect.width() / 16.0).ceil() as i32 {
                painter.rect_filled(
                    egui::Rect::from_min_size(rect.min + Vec2::new(x as f32 * 16.0, y as f32 * 16.0), Vec2::splat(16.0)),
                    0.0,
                    if (x + y) % 2 == 0 { p.field } else { p.raised },
                );
            }
        }
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta());
            view.pan[0] += delta.x;
            view.pan[1] += delta.y;
        }
        if response.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                view.distance = (view.distance * (-wheel * 0.002).exp()).clamp(0.2, 20.0);
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        if response.double_clicked() {
            view.distance = 3.0;
            view.pan = [0.0; 2];
        }
        let scale = (rect.width() / size.x).min(rect.height() / size.y) * 3.0 / view.distance;
        let image_rect = egui::Rect::from_center_size(rect.center() + Vec2::from(view.pan), size * scale);
        painter.image(
            texture.id(),
            image_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        ui.spinner();
    }
}

fn animation_controls(ui: &mut egui::Ui, p: &Palette, animation: &mut Animation, bvh: Option<&BvhInfo>) {
    widgets::section_header(ui, p, "person-simple-run", aurora_i18n::tr!("import-animation"));
    ui.label(aurora_i18n::tr!(
        "import-animation-info",
        duration = format!("{:.3}", animation.duration),
        joints = animation.joints.len()
    ));
    if let Some(bvh) = bvh {
        ui.label(aurora_i18n::tr!(
            "import-animation-frames",
            frames = bvh.frames,
            rate = format!("{:.2}", 1.0 / bvh.frame_time)
        ));
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("import-priority"));
            if ui.add(egui::DragValue::new(&mut animation.base_priority).range(0..=6)).changed() {
                for joint in &mut animation.joints {
                    joint.priority = animation.base_priority;
                }
            }
        });
        ui.checkbox(&mut animation.looping, aurora_i18n::tr!("import-loop"));
        let hand_labels = super::animation::hands();
        egui::ComboBox::from_id_salt("animation_hand")
            .selected_text(hand_labels[(animation.hand_pose as usize).min(12)])
            .show_ui(ui, |ui| {
                for (i, label) in hand_labels.into_iter().enumerate() {
                    ui.selectable_value(&mut animation.hand_pose, i as u32, label);
                }
            });
        let emotes = super::animation::emotes();
        egui::ComboBox::from_id_salt("animation_emote")
            .selected_text(
                emotes
                    .iter()
                    .find(|(name, _, _)| *name == animation.emote_name)
                    .map(|(_, _, label)| *label)
                    .unwrap_or(aurora_i18n::tr!("import-emote-none")),
            )
            .show_ui(ui, |ui| {
                for (name, _, label) in emotes {
                    ui.selectable_value(&mut animation.emote_name, name.into(), label);
                }
            });
        ui.add_enabled_ui(animation.looping, |ui| {
            for (label, value) in [
                (aurora_i18n::tr!("import-loop-in"), &mut animation.loop_in),
                (aurora_i18n::tr!("import-loop-out"), &mut animation.loop_out),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(
                        egui::DragValue::new(value)
                            .range(0.0..=animation.duration)
                            .suffix(" s")
                            .speed(bvh.frame_time),
                    );
                    let mut percent = if animation.duration > 0.0 {
                        *value / animation.duration * 100.0
                    } else {
                        0.0
                    };
                    if ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).suffix(" %")).changed() {
                        *value = percent / 100.0 * animation.duration;
                    }
                    let mut frame = (*value / bvh.frame_time).round() as usize;
                    if ui
                        .add(egui::DragValue::new(&mut frame).range(0..=bvh.frames.saturating_sub(2)))
                        .changed()
                    {
                        *value = frame as f32 * bvh.frame_time;
                    }
                });
            }
            animation.loop_out = animation.loop_out.max(animation.loop_in);
        });
        for (label, value) in [
            (aurora_i18n::tr!("import-ease-in"), &mut animation.ease_in),
            (aurora_i18n::tr!("import-ease-out"), &mut animation.ease_out),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::DragValue::new(value).range(0.0..=animation.duration).suffix(" s").speed(0.01));
            });
        }
    } else {
        ui.label(RichText::new(aurora_i18n::tr!("import-anim-original")).color(p.muted));
    }
}

fn material(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    material: &mut PbrMaterial,
    images: &mut [Option<LocalImage>; 4],
    actions: &mut Vec<Action>,
) {
    widgets::section_header(ui, p, "cube", aurora_i18n::tr!("import-material"));
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-base-color"));
        ui.color_edit_button_rgba_unmultiplied(&mut material.base_color_factor);
    });
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-emissive"));
        ui.color_edit_button_rgb(&mut material.emissive_factor);
    });
    ui.add(egui::Slider::new(&mut material.metallic_factor, 0.0..=1.0).text(aurora_i18n::tr!("import-metallic")));
    ui.add(egui::Slider::new(&mut material.roughness_factor, 0.0..=1.0).text(aurora_i18n::tr!("import-roughness")));
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-alpha-mode"));
        egui::ComboBox::from_id_salt("import_alpha")
            .selected_text(alpha_name(material.alpha_mode))
            .show_ui(ui, |ui| {
                for mode in [
                    aurora_assets::AlphaMode::Opaque,
                    aurora_assets::AlphaMode::Blend,
                    aurora_assets::AlphaMode::Mask,
                ] {
                    ui.selectable_value(&mut material.alpha_mode, mode, alpha_name(mode));
                }
            });
    });
    if material.alpha_mode == aurora_assets::AlphaMode::Mask {
        ui.add(egui::Slider::new(&mut material.alpha_cutoff, 0.0..=1.0).text(aurora_i18n::tr!("import-alpha-cutoff")));
    }
    ui.checkbox(&mut material.double_sided, aurora_i18n::tr!("import-double-sided"));
    ui.add_space(8.0);
    for (slot, label) in [
        aurora_i18n::tr!("import-base-color"),
        aurora_i18n::tr!("import-normal"),
        aurora_i18n::tr!("import-metallic-roughness"),
        aurora_i18n::tr!("import-emissive"),
    ]
    .into_iter()
    .enumerate()
    {
        ui.horizontal(|ui| {
            ui.label(label);
            if let Some(image) = &images[slot] {
                ui.label(&image.name)
                    .on_hover_text(format!("{} × {}", image.pixels.width, image.pixels.height));
            } else {
                if let Some(asset) = maps::ids(material)[slot] {
                    ui.label(asset.to_string());
                } else {
                    ui.label(RichText::new(aurora_i18n::tr!("import-no-local-image")).color(p.muted));
                }
            }
            if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                actions.push(Action::MapFile(id, slot));
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-inventory")).clicked() {
                actions.push(Action::MapInventory(id, slot));
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-remove")).clicked() {
                images[slot] = None;
                match slot {
                    0 => material.base_color_texture = None,
                    1 => material.normal_texture = None,
                    2 => material.metallic_roughness_texture = None,
                    _ => material.emissive_texture = None,
                }
            }
        });
    }
}
fn alpha_name(mode: aurora_assets::AlphaMode) -> &'static str {
    match mode {
        aurora_assets::AlphaMode::Opaque => aurora_i18n::tr!("import-alpha-opaque"),
        aurora_assets::AlphaMode::Blend => aurora_i18n::tr!("import-alpha-blend"),
        aurora_assets::AlphaMode::Mask => aurora_i18n::tr!("import-alpha-mask"),
    }
}
