//! Aurora import windows: stable layout, short labels, scoped advanced
//! controls and a persistent footer showing the exact confirmed price.

use super::*;
use crate::{
    theme::Palette,
    ui::widgets::{self, Floater, flat_button},
};
use egui::{RichText, Vec2};

pub(super) mod destination;
mod mesh_panel;
#[cfg(test)]
mod tests;

// egui's menu style and button strokes can change the padding between states.
// Keep popup row geometry fixed while still using native keyboard interaction.
fn selection<T: PartialEq>(ui: &mut egui::Ui, current: &mut T, value: T, label: impl Into<egui::WidgetText>) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
            let w = &mut ui.visuals_mut().widgets;
            for visuals in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                visuals.bg_stroke = egui::Stroke::NONE;
                visuals.expansion = 0.0;
            }
            ui.add_sized(
                Vec2::new(ui.available_width(), ui.spacing().interact_size.y),
                egui::Button::new(label.into())
                    .selected(*current == value)
                    .frame(true)
                    .frame_when_inactive(true)
                    .truncate(),
            )
        })
        .inner;
    if response.clicked() && *current != value {
        *current = value;
        let mut response = response;
        response.mark_changed();
        response
    } else {
        response
    }
}

fn checkbox(ui: &mut egui::Ui, p: &Palette, checked: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for visuals in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            visuals.corner_radius = egui::CornerRadius::same(2);
            visuals.bg_stroke = egui::Stroke::new(1.0, p.muted);
            visuals.bg_fill = if *checked { p.violet } else { p.field };
        }
        ui.checkbox(checked, label.into())
    })
    .inner
}

fn column_label(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, width: f32) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(width, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(label.into()).truncate())
    })
    .inner
}

pub enum Action {
    Files,
    Start,
    Cancel,
    Preview(Uuid),
    Play(Uuid, bool),
    Reload(Uuid),
    RestoreDraft(Uuid),
    ResetMesh(Uuid),
    Quote(Uuid),
    MeshFile(Uuid, usize),
    PhysicsPreset(Uuid, bool),
    Open(Uuid),
    Local(Uuid),
    ClearLocalMesh,
    RezLocalMesh(Uuid),
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

fn file_status(ui: &mut egui::Ui, p: &Palette, entry: &Entry) {
    let response = ui
        .allocate_ui_with_layout(
            Vec2::new(18.0, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| match &entry.status {
                Status::Ready => {}
                Status::Preparing | Status::Uploading => {
                    ui.add(egui::Spinner::new().size(16.0));
                }
                Status::Done(_) => widgets::glyph(ui, "check", p.success, 16.0),
                Status::Cancelled => widgets::glyph(ui, "x", p.muted, 16.0),
                Status::Error(_) => widgets::glyph(ui, "warning", p.danger, 16.0),
            },
        )
        .response;
    if !matches!(entry.status, Status::Ready) {
        response.on_hover_text(status(entry));
    }
}

fn metadata_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    inventory: &crate::world::inventory::Inventory,
    protected: &HashSet<Uuid>,
    picker: &mut destination::Picker,
    demo: bool,
) {
    egui::Grid::new("import_metadata")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 8.0))
        .show(ui, |ui| {
            ui.label(aurora_i18n::tr!("common-name"));
            ui.add(egui::TextEdit::singleline(&mut entry.name).desired_width(360.0).char_limit(63));
            ui.end_row();
            if !entry.local {
                ui.label(aurora_i18n::tr!("common-description"));
                ui.add(
                    egui::TextEdit::singleline(&mut entry.description)
                        .desired_width(360.0)
                        .char_limit(127),
                );
                ui.end_row();
                ui.label(aurora_i18n::tr!("import-destination"));
                if demo {
                    picker.demo(ui);
                }
                picker.show(ui, p, inventory, protected, &mut entry.folder);
                ui.end_row();
            }
        });
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
    let mesh_layout = imports.choice == Choice::Mesh
        || (imports.choice == Choice::Local && prefs.local_meshes)
        || imports
            .active
            .and_then(|id| imports.entries.iter().find(|e| e.id == id))
            .is_some_and(|e| matches!(e.source, Some(Source::Mesh(_))));
    let material_layout = imports.choice == Choice::Material
        || imports
            .active
            .and_then(|id| imports.entries.iter().find(|e| e.id == id))
            .is_some_and(|e| matches!(e.source, Some(Source::Material { .. })));
    let mut floater = Floater::new(
        "asset_imports_v3",
        aurora_i18n::tr!("import-title"),
        egui::pos2(100.0, 80.0),
        if mesh_layout {
            Vec2::new(1080.0, 740.0)
        } else if material_layout {
            Vec2::new(980.0, 740.0)
        } else {
            Vec2::new(840.0, 650.0)
        },
    );
    floater.min_size = if mesh_layout {
        Vec2::new(980.0, 700.0)
    } else if material_layout {
        Vec2::new(900.0, 700.0)
    } else {
        Vec2::new(720.0, 500.0)
    };
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
        if imports.choice == Choice::Animation
            || imports
                .active
                .and_then(|id| imports.entries.iter().find(|entry| entry.id == id))
                .is_some_and(|entry| matches!(entry.source, Some(Source::Animation { .. })))
        {
            checkbox(ui, p, &mut prefs.preview_on_avatar, aurora_i18n::tr!("import-preview-own-avatar"));
        }
        if imports.choice == Choice::Local {
            ui.horizontal_wrapped(|ui| {
                checkbox(ui, p, &mut prefs.local_meshes, aurora_i18n::tr!("import-local-mesh-files"));
                if prefs.local_meshes {
                    checkbox(ui, p, &mut prefs.local_assume_meters, aurora_i18n::tr!("import-local-meters"));
                    checkbox(ui, p, &mut prefs.local_mesh_auto_reload, aurora_i18n::tr!("import-local-live"));
                    ui.add(
                        egui::DragValue::new(&mut prefs.local_reload_seconds)
                            .range(0.1..=3600.0)
                            .suffix(" s"),
                    );
                }
            });
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && imports.pending_files.is_none(),
                    egui::Button::new(if imports.choice == Choice::Mesh {
                        aurora_i18n::tr!("import-mesh-choose-high")
                    } else {
                        aurora_i18n::tr!("import-choose-files")
                    }),
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
                    checkbox(ui, p, &mut imports.scale_bulk_1k, aurora_i18n::tr!("import-limit-bulk-1k"));
                });
            }
        });
        // Reserve the actual footer, independently of the outer Window's
        // remembered size. Subtracting a fixed margin from that size creates
        // a growth loop when a receipt, message or permission row appears.
        let extra_rows = usize::from(!imports.message.is_empty()) + usize::from(prefs.show_payment && imports.spent > 0);
        let extra_height = extra_rows as f32 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
        let footer_base = ctx.data_mut(|d| d.get_temp::<f32>(window_id.with("footer_base"))).unwrap_or(90.0);
        let height = (ui.available_height() - footer_base - extra_height).max(100.0);
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
                                ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready | Status::Error(_)), |ui| {
                                    checkbox(ui, p, &mut entry.selected, "");
                                });
                                let name_width = (ui.available_width() - 18.0 - ui.spacing().item_spacing.x).max(40.0);
                                if ui
                                    .add_sized(
                                        Vec2::new(name_width, 26.0),
                                        egui::Button::selectable(imports.active == Some(entry.id), "")
                                            .left_text(&entry.name)
                                            .truncate(),
                                    )
                                    .on_hover_text(entry.path.to_string_lossy())
                                    .clicked()
                                {
                                    imports.active = Some(entry.id);
                                    if matches!(entry.source, Some(Source::Animation { .. })) {
                                        actions.push(Action::Play(entry.id, true));
                                    }
                                }
                                file_status(ui, p, entry);
                            });
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
                                entry.view.background = prefs.preview_background.unwrap_or_else(|| palette_color(p.field));
                                entry.view.edge_color = prefs.preview_edges.unwrap_or_else(|| palette_color(p.teal));
                                entry.view.ambient = [0.2; 3];
                                entry.view.diagnostic_colors = [
                                    linear_color(prefs.preview_degenerate_fill.unwrap_or_else(|| palette_color(p.danger))),
                                    prefs.preview_degenerate_edges.unwrap_or_else(|| palette_color(p.warn)),
                                ];
                                let metadata = (entry.name.clone(), entry.description.clone(), entry.folder);
                                let preview_generation = entry.preview_generation;
                                let preview_error = entry.preview_error.clone();
                                if entry.local {
                                    checkbox(ui, p, &mut entry.local_live, aurora_i18n::tr!("import-local-live"));
                                }
                                if let Status::Done(item) = entry.status
                                    && flat_button(ui, p, aurora_i18n::tr!("import-open-inventory")).clicked()
                                {
                                    actions.push(Action::Open(item));
                                }
                                if entry.source.is_some()
                                    && matches!(entry.status, Status::Error(_))
                                    && ui
                                        .add_enabled(!busy, egui::Button::new(aurora_i18n::tr!("import-restore-draft")))
                                        .clicked()
                                {
                                    actions.push(Action::RestoreDraft(entry.id));
                                }
                                ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready), |ui| {
                                    if !matches!(entry.source, Some(Source::Mesh(_))) {
                                        metadata_controls(
                                            ui,
                                            p,
                                            entry,
                                            &world.inventory,
                                            &inventory_prefs.protected,
                                            &mut imports.destination,
                                            imports.demo_mode == "destination",
                                        );
                                        ui.add_space(12.0);
                                    }
                                    if matches!(entry.source, Some(Source::Mesh(_))) {
                                        mesh_panel::show(ui, p, entry, prefs, &mut actions, |ui, entry| {
                                            metadata_controls(
                                                ui,
                                                p,
                                                entry,
                                                &world.inventory,
                                                &inventory_prefs.protected,
                                                &mut imports.destination,
                                                imports.demo_mode == "destination",
                                            )
                                        });
                                    } else if let Some(source) = &mut entry.source {
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
                                                ui.horizontal(|ui| {
                                                    if flat_button(ui, p, aurora_i18n::tr!("common-play")).clicked() {
                                                        actions.push(Action::Play(entry.id, true));
                                                    }
                                                    if flat_button(ui, p, aurora_i18n::tr!("common-stop")).clicked() {
                                                        actions.push(Action::Play(entry.id, false));
                                                    }
                                                });
                                            }
                                            Source::Animation { animation, bvh, .. } => {
                                                let old_base = entry.player.base;
                                                let bases = [
                                                    aurora_i18n::tr!("import-base-standing"),
                                                    aurora_i18n::tr!("import-base-walking"),
                                                    aurora_i18n::tr!("import-base-sitting"),
                                                    aurora_i18n::tr!("import-base-flying"),
                                                ];
                                                ui.horizontal(|ui| {
                                                    ui.label(aurora_i18n::tr!("import-animation-base"));
                                                    egui::ComboBox::from_id_salt("animation_base")
                                                        .selected_text(bases[entry.player.base.min(3)])
                                                        .show_ui(ui, |ui| {
                                                            for (i, label) in bases.into_iter().enumerate() {
                                                                selection(ui, &mut entry.player.base, i, label);
                                                            }
                                                        });
                                                })
                                                .response
                                                .on_hover_text(aurora_i18n::tr!("import-animation-base-help"));
                                                animation_transport(ui, p, entry.id, &mut entry.player, animation, &mut actions);
                                                if let Some(model) = &entry.model {
                                                    aurora_render::preview::paint_with_pose(
                                                        ui,
                                                        egui::Id::new(("animation_preview", entry.id)),
                                                        model.clone(),
                                                        &mut entry.view,
                                                        Vec2::new(ui.available_width(), 180.0),
                                                        entry.player.pose.clone(),
                                                    );
                                                }
                                                if old_base != entry.player.base {
                                                    entry.player.seek(animation.local_time(f64::from(entry.player.position)) as f32);
                                                }
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
                                                let mut header = super::animation::metadata(animation);
                                                animation_controls(ui, p, &mut header, bvh.as_ref(), animation.joints.len());
                                                let changed = before
                                                    != (
                                                        header.base_priority,
                                                        header.looping,
                                                        header.loop_in,
                                                        header.loop_out,
                                                        header.ease_in,
                                                        header.ease_out,
                                                        header.hand_pose,
                                                        header.emote_name.clone(),
                                                    );
                                                if changed {
                                                    if animation.hand_pose != header.hand_pose || animation.emote_name != header.emote_name
                                                    {
                                                        entry.model = None;
                                                        entry.preview_generation += 1;
                                                        entry.preview_error = None;
                                                    }
                                                    let current = Arc::make_mut(animation);
                                                    header.joints = std::mem::take(&mut current.joints);
                                                    if current.base_priority != header.base_priority {
                                                        for joint in &mut header.joints {
                                                            joint.priority = header.base_priority;
                                                        }
                                                    }
                                                    *current = header;
                                                }
                                                if changed && entry.player.bound.is_some() {
                                                    actions.push(Action::Play(entry.id, true));
                                                }
                                            }
                                            Source::Material { value, images } => {
                                                entry.view.material = Some(super::preview::material(value));
                                                let old = value.clone();
                                                let old_maps = images.each_ref().map(|image| image.as_ref().map(|i| i.key));
                                                material(
                                                    ui,
                                                    p,
                                                    entry.id,
                                                    value,
                                                    images,
                                                    &mut actions,
                                                    entry.model.as_ref().map(|model| (model, &mut entry.view)),
                                                );
                                                if old != *value || old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key))
                                                {
                                                    entry.preview_generation += 1;
                                                }
                                                if old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key)) {
                                                    entry.model = None;
                                                    entry.preview_error = None;
                                                }
                                            }
                                            Source::Mesh(_) => {}
                                        }
                                    }
                                });
                                if metadata != (entry.name.clone(), entry.description.clone(), entry.folder) {
                                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                                        draft.invalidate_quote();
                                    }
                                    entry.preview_generation += 1;
                                }
                                if entry.preview_generation != preview_generation
                                    && let Some(error) = preview_error
                                    && imports.message == error
                                    && entry.preview_error.is_none()
                                {
                                    imports.message.clear();
                                }
                                if !matches!(entry.source, Some(Source::Sound(_) | Source::Mesh(_))) {
                                    preview_colors(ui, p, prefs);
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
        let footer_start = ui.cursor().top();
        ui.separator();
        if imports.choice != Choice::Local {
            ui.add_enabled_ui(!busy, |ui| {
                ui.collapsing(aurora_i18n::tr!("import-permissions"), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("inventory-permissions-next-owner"));
                        checkbox(ui, p, &mut prefs.next_modify, aurora_i18n::tr!("inventory-permissions-modify"));
                        checkbox(ui, p, &mut prefs.next_copy, aurora_i18n::tr!("inventory-permissions-copy"));
                        if !prefs.next_copy {
                            prefs.next_transfer = true;
                        }
                        ui.add_enabled(
                            prefs.next_copy,
                            egui::Checkbox::new(&mut prefs.next_transfer, aurora_i18n::tr!("inventory-permissions-transfer")),
                        );
                    });
                    ui.horizontal(|ui| {
                        checkbox(ui, p, &mut prefs.share_group, aurora_i18n::tr!("import-share-group"));
                        checkbox(ui, p, &mut prefs.everyone_copy, aurora_i18n::tr!("import-everyone-copy"));
                        checkbox(ui, p, &mut prefs.show_payment, aurora_i18n::tr!("import-show-payment"));
                    });
                });
            });
        }
        if !imports.message.is_empty() {
            ui.label(RichText::new(&imports.message).color(p.danger));
        }
        if prefs.show_payment && imports.spent > 0 {
            ui.label(RichText::new(aurora_i18n::tr!("import-paid", cost = imports.spent)).color(p.success));
        }
        ui.horizontal_wrapped(|ui| {
            let total = imports.total(&prices);
            let local = imports.choice == Choice::Local;
            ui.label(
                RichText::new(if local {
                    aurora_i18n::tr!("import-local-free").into()
                } else {
                    match total {
                        Some(cost) => aurora_i18n::tr!("import-total", cost = cost),
                        None => {
                            if imports.choice == Choice::Mesh {
                                aurora_i18n::tr!("import-mesh-fee-required").into()
                            } else {
                                aurora_i18n::tr!("import-price-unavailable").into()
                            }
                        }
                    }
                })
                .strong(),
            );
            let selected = imports.entries.iter().any(|e| e.selected && matches!(e.status, Status::Ready));
            let can_upload = !busy && selected && total.is_some_and(|cost| cost == 0 || world.balance.is_some_and(|b| b >= cost));
            if local {
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && ui
                        .add_enabled(
                            !entry.local_pending && entry.source.is_some(),
                            egui::Button::new(if matches!(entry.source, Some(Source::Mesh(_))) {
                                aurora_i18n::tr!("import-local-mesh-apply")
                            } else {
                                aurora_i18n::tr!("import-local-apply")
                            }),
                        )
                        .clicked()
                {
                    actions.push(Action::Local(entry.id));
                }
                if imports
                    .active
                    .and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    .is_some_and(|e| matches!(e.source, Some(Source::Mesh(_))))
                    && flat_button(ui, p, aurora_i18n::tr!("import-local-mesh-clear")).clicked()
                {
                    actions.push(Action::ClearLocalMesh);
                }
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && matches!(entry.source, Some(Source::Mesh(_)))
                    && flat_button(ui, p, aurora_i18n::tr!("import-local-mesh-rez")).clicked()
                {
                    actions.push(Action::RezLocalMesh(entry.id));
                }
            } else {
                if imports.choice == Choice::Mesh
                    && let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && let Some(Source::Mesh(draft)) = &entry.source
                    && ui
                        .add_enabled(
                            !busy && draft.pending.is_none(),
                            egui::Button::new(aurora_i18n::tr!("import-calculate-fee")),
                        )
                        .clicked()
                {
                    actions.push(Action::Quote(entry.id));
                }
                if ui
                    .add_enabled(can_upload, egui::Button::new(aurora_i18n::tr!("import-upload")))
                    .clicked()
                {
                    imports.confirm = true;
                }
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
        ctx.data_mut(|d| {
            d.insert_temp(
                window_id.with("footer_base"),
                (ui.cursor().top() - footer_start - extra_height).max(0.0),
            )
        });
    });
    imports.open = open && !close_requested;
    if prefs.permissions() != old_permissions.permissions() {
        for entry in &mut imports.entries {
            if let Some(Source::Mesh(draft)) = &mut entry.source {
                draft.invalidate_quote();
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

// LLFloaterModelPreview color swatches; Aurora's palette supplies defaults
// so previews follow the active theme until a color is explicitly saved.
fn preview_colors(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    ui.collapsing(aurora_i18n::tr!("import-preview-colors"), |ui| {
        for (label, stored, default) in [
            (
                aurora_i18n::tr!("import-preview-background"),
                &mut prefs.preview_background,
                p.field,
            ),
            (aurora_i18n::tr!("import-edges"), &mut prefs.preview_edges, p.teal),
            (
                aurora_i18n::tr!("import-preview-physics-fill"),
                &mut prefs.preview_physics_fill,
                p.teal,
            ),
            (
                aurora_i18n::tr!("import-preview-physics-edges"),
                &mut prefs.preview_physics_edges,
                p.ink,
            ),
            (
                aurora_i18n::tr!("import-preview-degenerate-fill"),
                &mut prefs.preview_degenerate_fill,
                p.danger,
            ),
            (
                aurora_i18n::tr!("import-preview-degenerate-edges"),
                &mut prefs.preview_degenerate_edges,
                p.warn,
            ),
        ] {
            ui.horizontal(|ui| {
                let mut color = color32(stored.unwrap_or_else(|| palette_color(default)));
                if ui.color_edit_button_srgba(&mut color).changed() {
                    *stored = Some(palette_color(color));
                }
                ui.label(label);
            });
        }
        if flat_button(ui, p, aurora_i18n::tr!("common-reset")).clicked() {
            prefs.preview_background = None;
            prefs.preview_edges = None;
            prefs.preview_physics_fill = None;
            prefs.preview_physics_edges = None;
            prefs.preview_degenerate_fill = None;
            prefs.preview_degenerate_edges = None;
        }
    });
}

fn palette_color(color: egui::Color32) -> [f32; 4] {
    color.to_srgba_unmultiplied().map(|v| f32::from(v) / 255.0)
}
fn color32(color: [f32; 4]) -> egui::Color32 {
    let [r, g, b, a] = color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
fn linear_color(color: [f32; 4]) -> [f32; 4] {
    // The egui preview target stores gamma RGB. PBR factors use linear RGB.
    egui::Rgba::from(color32(color)).to_rgba_unmultiplied()
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
                selection(ui, target, index, label);
            }
        });
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-max-size"));
        for (axis, value) in [("X", &mut options.max_width), ("Y", &mut options.max_height)] {
            egui::ComboBox::from_id_salt(("import_dimension", axis))
                .selected_text(value.to_string())
                .show_ui(ui, |ui| {
                    for size in [4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048] {
                        selection(ui, value, size, size.to_string());
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
        checkbox(ui, p, &mut options.lossless_small, aurora_i18n::tr!("import-lossless"));
    }
    if empty_alpha {
        checkbox(ui, p, &mut options.strip_alpha, aurora_i18n::tr!("import-strip-alpha"));
    }
    if old != *options || old_target != *target {
        *generation += 1;
        actions.push(Action::Preview(id));
    }
    if *target != 0 {
        if let Some(model) = model {
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
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), egui::Sense::drag());
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

fn animation_transport(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    player: &mut super::animation::Player,
    animation: &Animation,
    actions: &mut Vec<Action>,
) -> f32 {
    ui.horizontal(|ui| {
        if flat_button(ui, p, aurora_i18n::tr!("common-play")).clicked() {
            actions.push(Action::Play(id, true));
        }
        if flat_button(ui, p, aurora_i18n::tr!("common-stop")).clicked() {
            player.playing = false;
            player.seek(0.0);
            actions.push(Action::Play(id, false));
        }
        if flat_button(ui, p, aurora_i18n::tr!("import-pause")).clicked() {
            player.playing = !player.playing;
        }
        if flat_button(ui, p, aurora_i18n::tr!("import-reload")).clicked() {
            actions.push(Action::Reload(id));
        }
    });
    let mut position = animation.local_time(f64::from(player.position)) as f32;
    if ui
        .add(egui::Slider::new(&mut position, 0.0..=animation.duration).suffix(" s"))
        .changed()
    {
        player.seek(position);
    }
    position
}

fn animation_controls(ui: &mut egui::Ui, p: &Palette, animation: &mut Animation, bvh: Option<&BvhInfo>, joint_count: usize) {
    widgets::section_header(ui, p, "person-simple-run", aurora_i18n::tr!("import-animation"));
    ui.label(aurora_i18n::tr!(
        "import-animation-info",
        duration = format!("{:.3}", animation.duration),
        joints = joint_count
    ));
    if let Some(bvh) = bvh {
        ui.label(aurora_i18n::tr!(
            "import-animation-frames",
            frames = bvh.frames,
            rate = format!("{:.2}", 1.0 / bvh.frame_time)
        ));
        ui.collapsing(aurora_i18n::tr!("import-animation-settings"), |ui| {
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("import-priority"));
                if ui.add(egui::DragValue::new(&mut animation.base_priority).range(0..=6)).changed() {
                    for joint in &mut animation.joints {
                        joint.priority = animation.base_priority;
                    }
                }
            });
            checkbox(ui, p, &mut animation.looping, aurora_i18n::tr!("import-loop"));
            let hand_labels = super::animation::hands();
            egui::ComboBox::from_id_salt("animation_hand")
                .selected_text(hand_labels[(animation.hand_pose as usize).min(13)])
                .show_ui(ui, |ui| {
                    for (i, label) in hand_labels.into_iter().enumerate() {
                        selection(ui, &mut animation.hand_pose, i as u32, label);
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
                        selection(ui, &mut animation.emote_name, name.into(), label);
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
        });
    }
}

fn material(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    material: &mut PbrMaterial,
    images: &mut [Option<LocalImage>; 4],
    actions: &mut Vec<Action>,
    preview: Option<(&Arc<aurora_render::preview::Model>, &mut aurora_render::preview::View)>,
) {
    widgets::section_header(ui, p, "cube", aurora_i18n::tr!("import-material"));
    ui.horizontal_top(|ui| {
        if let Some((model, view)) = preview {
            aurora_render::preview::paint(
                ui,
                egui::Id::new(("material_preview", id)),
                model.clone(),
                view,
                Vec2::new(210.0, 210.0),
            );
            ui.add_space(8.0);
        }
        ui.vertical(|ui| {
            egui::Grid::new(("material_properties", id))
                .num_columns(2)
                .min_col_width(140.0)
                .spacing(Vec2::new(8.0, 6.0))
                .show(ui, |ui| {
                    ui.label(aurora_i18n::tr!("import-base-color"));
                    ui.color_edit_button_rgba_unmultiplied(&mut material.base_color_factor);
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("import-emissive"));
                    ui.color_edit_button_rgb(&mut material.emissive_factor);
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("import-metallic"));
                    ui.add(egui::Slider::new(&mut material.metallic_factor, 0.0..=1.0));
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("import-roughness"));
                    ui.add(egui::Slider::new(&mut material.roughness_factor, 0.0..=1.0));
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("import-alpha-mode"));
                    egui::ComboBox::from_id_salt("import_alpha")
                        .selected_text(alpha_name(material.alpha_mode))
                        .show_ui(ui, |ui| {
                            for mode in [
                                aurora_assets::AlphaMode::Opaque,
                                aurora_assets::AlphaMode::Blend,
                                aurora_assets::AlphaMode::Mask,
                            ] {
                                selection(ui, &mut material.alpha_mode, mode, alpha_name(mode));
                            }
                        });
                    ui.end_row();
                    if material.alpha_mode == aurora_assets::AlphaMode::Mask {
                        ui.label(aurora_i18n::tr!("import-alpha-cutoff"));
                        ui.add(egui::Slider::new(&mut material.alpha_cutoff, 0.0..=1.0));
                        ui.end_row();
                    }
                });
            checkbox(ui, p, &mut material.double_sided, aurora_i18n::tr!("import-double-sided"));
        });
    });
    ui.add_space(8.0);
    let name_width = (ui.available_width() - 408.0).max(80.0);
    egui::Grid::new(("material_maps", id))
        .num_columns(5)
        .spacing(Vec2::new(8.0, 6.0))
        .show(ui, |ui| {
            for (slot, label) in [
                aurora_i18n::tr!("import-base-color"),
                aurora_i18n::tr!("import-normal"),
                aurora_i18n::tr!("import-metallic-roughness"),
                aurora_i18n::tr!("import-emissive"),
            ]
            .into_iter()
            .enumerate()
            {
                column_label(ui, label, 140.0);
                if let Some(image) = &images[slot] {
                    column_label(ui, &image.name, name_width)
                        .on_hover_text(format!("{}\n{} × {}", image.name, image.pixels.width, image.pixels.height));
                } else {
                    if let Some(asset) = maps::ids(material)[slot] {
                        column_label(ui, asset.to_string(), name_width).on_hover_text(asset.to_string());
                    } else {
                        column_label(
                            ui,
                            RichText::new(aurora_i18n::tr!("import-no-local-image")).color(p.muted),
                            name_width,
                        );
                    }
                }
                if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("import-browse"), Vec2::new(80.0, 26.0)).clicked() {
                    actions.push(Action::MapFile(id, slot));
                }
                if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-inventory"), Vec2::new(82.0, 26.0)).clicked() {
                    actions.push(Action::MapInventory(id, slot));
                }
                if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-remove"), Vec2::new(66.0, 26.0)).clicked() {
                    images[slot] = None;
                    match slot {
                        0 => material.base_color_texture = None,
                        1 => material.normal_texture = None,
                        2 => material.metallic_roughness_texture = None,
                        _ => material.emissive_texture = None,
                    }
                }
                ui.end_row();
            }
        });
}
fn alpha_name(mode: aurora_assets::AlphaMode) -> &'static str {
    match mode {
        aurora_assets::AlphaMode::Opaque => aurora_i18n::tr!("import-alpha-opaque"),
        aurora_assets::AlphaMode::Blend => aurora_i18n::tr!("import-alpha-blend"),
        aurora_assets::AlphaMode::Mask => aurora_i18n::tr!("import-alpha-mask"),
    }
}
