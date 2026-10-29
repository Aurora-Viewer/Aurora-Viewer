//! Aurora import windows: stable layout, short labels, scoped advanced
//! controls and a persistent footer showing the exact confirmed price.

use super::*;
use crate::{
    theme::Palette,
    ui::widgets::{self, ActionKind, Floater, action_button, action_button_if, checkbox, flat_button},
};
use egui::{RichText, Vec2};

const IMPORT_POSITION: egui::Pos2 = egui::Pos2 { x: 48.0, y: 80.0 };

pub(super) mod destination;
mod inspector;
pub(crate) mod launcher;
mod material_panel;
mod mesh_panel;
pub(crate) mod permission_panel;
mod sound_panel;
#[cfg(test)]
mod tests;
mod zoom;

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

/// Collapsible section under its title: the shared header, with the id a
/// stock `ui.collapsing` gave to the same title.
fn fold(ui: &mut egui::Ui, p: &Palette, title: &str, default_open: bool, body: impl FnOnce(&mut egui::Ui)) {
    widgets::collapsing(ui, p, title, title, default_open, body);
}

/// A flat text button of the toolbar, greyed out when `enabled` is false.
fn tool_button(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| flat_button(ui, p, label)).inner
}

/// Swatch and picker of a glTF factor: linear components with a straight
/// alpha, edited as the sRGB bytes of the shared color picker. The factor is
/// written only when the user changes it, so an untouched one keeps its
/// exact value.
pub(crate) fn factor_swatch(ui: &mut egui::Ui, p: &Palette, id_salt: impl egui::AsIdSalt, factor: &mut [f32], alpha: bool) {
    use egui::ecolor::{gamma_u8_from_linear_f32, linear_f32_from_gamma_u8, linear_u8_from_linear_f32};
    let mut srgba = [255u8; 4];
    for (byte, linear) in srgba.iter_mut().zip(factor.iter()).take(3) {
        *byte = gamma_u8_from_linear_f32(*linear);
    }
    if let Some(a) = factor.get(3) {
        srgba[3] = linear_u8_from_linear_f32(*a);
    }
    if widgets::color_button(ui, p, id_salt, &mut srgba, alpha).changed() {
        for (linear, byte) in factor.iter_mut().zip(srgba).take(3) {
            *linear = linear_f32_from_gamma_u8(byte);
        }
        if let Some(a) = factor.get_mut(3) {
            *a = f32::from(srgba[3]) / 255.0;
        }
    }
}

pub enum Action {
    Files,
    Start,
    Cancel,
    Preview(Uuid),
    Play(Uuid, bool),
    AnimationModel(Uuid),
    DefaultAnimationModel(Uuid),
    SoundGain(Uuid),
    Reload(Uuid),
    RestoreDraft(Uuid),
    ResetMesh(Uuid),
    Quote(Uuid),
    MeshFile(Uuid, usize),
    PhysicsPreset(Uuid, bool),
    Open(Uuid),
    Local(Uuid),
    ClearLocal(Uuid),
    RezLocal(Uuid),
    MapFile(Uuid, usize),
    MapInventory(Uuid, usize),
    Zoom(Uuid, Option<usize>),
    LargeMap(Uuid, usize),
    Remove(Uuid),
    Resume,
}

fn server_message(message: &str) -> &str {
    // LLMeshUploadThread::log_upload_error also translates these MAV details.
    match message {
        "MAV_FOUND_DEGENERATE_TRIANGLES" => aurora_i18n::tr!("import-mesh-error-degenerate"),
        "MAV_CONFIRMATION_DATA_MISMATCH" => aurora_i18n::tr!("import-mesh-error-confirmation"),
        "MAV_BLOCK_MISSING" => aurora_i18n::tr!("import-mesh-error-missing"),
        "MAV_UNKNOWN_VERSION" => aurora_i18n::tr!("import-mesh-error-version"),
        _ => message,
    }
}

pub fn error(error: Error) -> String {
    if let Error::Refused(reason) | Error::Uncertain(reason) = &error {
        let mut details = Vec::new();
        if let Some(status) = reason.status {
            details.push(aurora_i18n::tr!("import-error-http", status = status));
        }
        if !reason.identifier.is_empty() {
            details.push(aurora_i18n::tr!("import-error-code", code = &reason.identifier));
        }
        let explanation = server_message(&reason.identifier);
        if explanation != reason.identifier {
            details.push(explanation.to_owned());
        }
        for message in &reason.messages {
            let message = server_message(message).to_owned();
            if !details.contains(&message) {
                details.push(message);
            }
        }
        if reason.messages.is_empty() && explanation == reason.identifier {
            details.push(aurora_i18n::tr!("import-error-no-details").into());
        }
        let heading = if matches!(error, Error::Uncertain(_)) {
            aurora_i18n::tr!("import-unknown-outcome")
        } else {
            aurora_i18n::tr!("import-rejected")
        };
        return format!("{heading}\n{}", details.join("\n"));
    }
    if let Error::PriceChanged(Some(cost)) = error {
        return aurora_i18n::tr!("import-new-price", cost = cost);
    }
    match error {
        Error::Invalid => aurora_i18n::tr!("import-invalid-request"),
        Error::Unavailable => aurora_i18n::tr!("import-service-unavailable"),
        Error::Rejected => aurora_i18n::tr!("import-rejected"),
        Error::MeshPermission => aurora_i18n::tr!("import-mesh-permission-denied"),
        Error::Refused(_) | Error::Uncertain(_) => unreachable!("details handled above"),
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
                Status::Error(_) => widgets::glyph(ui, "alert-triangle-filled", p.danger, 16.0),
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
    let mesh = matches!(entry.source, Some(Source::Mesh(_)));
    egui::Grid::new("import_metadata")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 8.0))
        .show(ui, |ui| {
            ui.label(aurora_i18n::tr!("common-name"));
            ui.add(
                egui::TextEdit::singleline(&mut entry.name)
                    .desired_width((ui.available_width() - if mesh { 0.0 } else { 100.0 }).clamp(120.0, 360.0))
                    .char_limit(63),
            );
            ui.end_row();
            if !entry.local {
                ui.label(aurora_i18n::tr!("common-description"));
                ui.add(
                    egui::TextEdit::singleline(&mut entry.description)
                        .desired_width((ui.available_width() - if mesh { 0.0 } else { 100.0 }).clamp(120.0, 360.0))
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

fn has_selected_mesh(imports: &Imports) -> bool {
    imports
        .entries
        .iter()
        .any(|entry| imports.choice.accepts(entry) && entry.selected && matches!(entry.source, Some(Source::Mesh(_))))
}

fn mesh_blocked(imports: &Imports, connected: bool, account: Uuid) -> bool {
    connected && has_selected_mesh(imports) && imports.mesh_permission_denied && imports.mesh_permission_account == account
}

fn price_label(imports: &Imports, prices: &Prices, connected: bool, account: Uuid) -> String {
    if imports.choice == Choice::Local {
        return aurora_i18n::tr!("import-local-free").into();
    }
    if !connected {
        return aurora_i18n::tr!("import-price-unavailable").into();
    }
    if mesh_blocked(imports, connected, account) {
        return aurora_i18n::tr!("import-mesh-account-denied").into();
    }
    if let Some(cost) = imports.total(prices) {
        return aurora_i18n::tr!("import-total", cost = cost);
    }
    if imports
        .entries
        .iter()
        .any(|entry| imports.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Error(_)))
    {
        return aurora_i18n::tr!("import-fix-errors-before-upload").into();
    }
    if has_selected_mesh(imports) {
        return aurora_i18n::tr!("import-mesh-fee-required").into();
    }
    aurora_i18n::tr!("import-price-pending").into()
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
    let old_permissions = prefs.clone();
    launcher::show(ctx, p, imports, prefs);
    if imports.open && std::mem::take(&mut imports.choose_files_on_open) {
        actions.push(Action::Files);
    }
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
    inspector::show(ctx, p, imports, prefs, world, inventory_prefs, &prices, &mut actions);
    if imports.launcher_open && imports.open {
        launcher::keep_accessible(ctx, imports.window_choice().window_id());
    }
    if prefs.permissions_for(Kind::Mesh) != old_permissions.permissions_for(Kind::Mesh) {
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
                let size = Vec2::new(0.0, 26.0);
                let upload = aurora_i18n::tr!("import-upload");
                if action_button(ui, p, "file-upload-filled", upload, ActionKind::Primary, size).clicked() {
                    actions.push(Action::Start);
                    imports.confirm = false;
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    imports.confirm = false;
                }
            });
        });
        if !confirm {
            imports.confirm = false;
        }
    }
    zoom::show(ctx, p, imports, &mut actions);
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
    fold(ui, p, aurora_i18n::tr!("import-preview-colors"), false, |ui| {
        preview_colors_fields(ui, p, prefs);
    });
}

fn preview_colors_fields(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
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
            // stored as straight sRGB components in 0..=1
            let mut srgba = color32(stored.unwrap_or_else(|| palette_color(default))).to_srgba_unmultiplied();
            let response = widgets::color_button(ui, p, ("import_preview_color", label), &mut srgba, true);
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique(("import_preview_swatch", label)), (response.id, response.rect)));
            if response.changed() {
                *stored = Some(srgba.map(|v| f32::from(v) / 255.0));
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
}

fn preview_colors_button(ui: &mut egui::Ui, p: &Palette, id: Uuid, prefs: &mut Preferences) {
    let key = egui::Id::unique(("import_preview_colors_open", id));
    let position = ui.next_widget_position();
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::unique(("import_preview_palette", id)),
            egui::Rect::from_min_size(position, Vec2::splat(20.0)),
        )
    });
    let mut open = ui.ctx().data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
    if widgets::icon_button(ui, p, "palette-filled", aurora_i18n::tr!("import-preview-colors"), true) {
        open = !open;
    }
    if open {
        let mut floater = Floater::new(
            "import_preview_colors",
            aurora_i18n::tr!("import-preview-colors"),
            position + Vec2::new(250.0, 24.0),
            Vec2::new(330.0, 310.0),
        )
        .fixed()
        .auto_height();
        floater.min_size = Vec2::new(300.0, 290.0);
        floater.show(ui.ctx(), p, &mut open, |ui| {
            preview_colors_fields(ui, p, prefs);
        });
        if open {
            ui.ctx()
                .move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("import_preview_colors")));
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(key, open));
}

fn apply_preview_colors(view: &mut aurora_render::preview::View, p: &Palette, prefs: &Preferences) {
    view.background = prefs.preview_background.unwrap_or_else(|| palette_color(p.field));
    view.edge_color = prefs.preview_edges.unwrap_or_else(|| palette_color(p.teal));
    view.ambient = [0.2; 3];
    view.diagnostic_colors = [
        linear_color(prefs.preview_degenerate_fill.unwrap_or_else(|| palette_color(p.danger))),
        prefs.preview_degenerate_edges.unwrap_or_else(|| palette_color(p.warn)),
    ];
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
    prefs: &mut Preferences,
    preview: &Option<egui::TextureHandle>,
    actions: &mut Vec<Action>,
    generation: &mut u64,
    target: &mut usize,
    model: &Option<Arc<aurora_render::preview::Model>>,
    view: &mut aurora_render::preview::View,
) {
    widgets::section_header(ui, p, "photo-filled", aurora_i18n::tr!("import-preview"));
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
    if texture_needs_size_limit(image) {
        let mut limit_size = options.max_width.max(options.max_height) <= 1024;
        if widgets::switch_labeled(ui, p, &mut limit_size, aurora_i18n::tr!("import-max-size")).changed() {
            let maximum = if limit_size { 1024 } else { 2048 };
            options.max_width = maximum;
            options.max_height = maximum;
            prefs.limit_texture_size = limit_size;
        }
    }
    let (width, height) = import_texture::dimensions(image.width, image.height, *options);
    ui.label(aurora_i18n::tr!("import-texture-size", width = width, height = height));
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
                egui::Id::unique(("texture_preview", id)),
                model.clone(),
                view,
                Vec2::new(ui.available_width(), 260.0),
            );
            preview_camera_controls(ui, p, view);
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
            widgets::PICTURE_TINT,
        );
    } else {
        ui.spinner();
    }
}

fn animation_model_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    model: &super::animation_model::Selection,
    actions: &mut Vec<Action>,
) {
    ui.label(aurora_i18n::tr!("import-animation-model"));
    let name = model
        .current
        .as_ref()
        .and_then(|model| model.path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| aurora_i18n::tr!("import-animation-model-default").into());
    ui.add(egui::Label::new(name).truncate())
        .on_hover_text(aurora_i18n::tr!("import-animation-model-help"));
    ui.horizontal(|ui| {
        let choose = tool_button(ui, p, aurora_i18n::tr!("import-animation-model-choose"), !model.pending);
        if choose.clicked() {
            actions.push(Action::AnimationModel(id));
        }
        let reset = tool_button(
            ui,
            p,
            aurora_i18n::tr!("import-animation-model-default"),
            model.current.is_some() || model.pending,
        );
        if reset.clicked() {
            actions.push(Action::DefaultAnimationModel(id));
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::unique(("animation_model_choose", id)), choose.rect);
            data.insert_temp(egui::Id::unique(("animation_model_reset", id)), reset.rect);
        });
        if model.pending {
            ui.spinner();
        }
    });
    if let Some(error) = &model.error {
        ui.label(RichText::new(error).small().color(p.danger));
    }
}

fn animation_transport(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    player: &mut super::animation::Player,
    animation: &Animation,
    preview_on_avatar: &mut bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.horizontal_wrapped(|ui| {
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
    });
    let mut position = animation.local_time(f64::from(player.position)) as f32;
    if ui
        .add(egui::Slider::new(&mut position, 0.0..=animation.duration).suffix(" s"))
        .changed()
    {
        player.seek(position);
    }
    checkbox(ui, p, preview_on_avatar, aurora_i18n::tr!("import-preview-own-avatar"));
    actions
}

fn animation_controls(ui: &mut egui::Ui, p: &Palette, animation: &mut Animation, bvh: Option<&BvhInfo>, joint_count: usize) {
    widgets::section_header(
        ui,
        p,
        "adjustments-filled",
        if bvh.is_some() {
            aurora_i18n::tr!("import-animation-settings")
        } else {
            aurora_i18n::tr!("import-animation")
        },
    );
    ui.label(
        RichText::new(aurora_i18n::tr!(
            "import-animation-info",
            duration = format!("{:.3}", animation.duration),
            joints = joint_count
        ))
        .small()
        .color(p.muted),
    );
    let Some(bvh) = bvh else {
        return;
    };
    ui.label(
        RichText::new(aurora_i18n::tr!(
            "import-animation-frames",
            frames = bvh.frames,
            rate = format!("{:.2}", 1.0 / bvh.frame_time)
        ))
        .small()
        .color(p.muted),
    );
    ui.add_space(12.0);
    egui::Grid::new("animation_identity")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 8.0))
        .show(ui, |ui| {
            ui.label(aurora_i18n::tr!("import-priority"));
            ui.add(egui::DragValue::new(&mut animation.base_priority).range(0..=6));
            ui.end_row();
            ui.label(aurora_i18n::tr!("import-animation-hands"));
            let hands = super::animation::hands();
            egui::ComboBox::from_id_salt("animation_hand")
                .width(180.0)
                .selected_text(hands[(animation.hand_pose as usize).min(13)])
                .show_ui(ui, |ui| {
                    for (index, label) in hands.into_iter().enumerate() {
                        selection(ui, &mut animation.hand_pose, index as u32, label);
                    }
                });
            ui.end_row();
            ui.label(aurora_i18n::tr!("import-animation-expression"));
            let emotes = super::animation::emotes();
            egui::ComboBox::from_id_salt("animation_emote")
                .width(180.0)
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
            ui.end_row();
        });
    ui.add_space(12.0);
    checkbox(ui, p, &mut animation.looping, aurora_i18n::tr!("import-loop"));
    ui.add_enabled_ui(animation.looping, |ui| {
        egui::Grid::new("animation_loop")
            .num_columns(4)
            .spacing(Vec2::new(10.0, 8.0))
            .show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new(aurora_i18n::tr!("import-animation-seconds")).small().color(p.muted));
                ui.label(RichText::new("%").small().color(p.muted));
                ui.label(RichText::new(aurora_i18n::tr!("import-animation-frame")).small().color(p.muted));
                ui.end_row();
                for (label, value) in [
                    (aurora_i18n::tr!("import-loop-in"), &mut animation.loop_in),
                    (aurora_i18n::tr!("import-loop-out"), &mut animation.loop_out),
                ] {
                    ui.label(label);
                    ui.add_sized(
                        Vec2::new(64.0, 26.0),
                        egui::DragValue::new(value)
                            .range(0.0..=animation.duration)
                            .speed(bvh.frame_time)
                            .fixed_decimals(3),
                    );
                    let mut percent = if animation.duration > 0.0 {
                        *value / animation.duration * 100.0
                    } else {
                        0.0
                    };
                    if ui
                        .add_sized(
                            Vec2::new(58.0, 26.0),
                            egui::DragValue::new(&mut percent).range(0.0..=100.0).fixed_decimals(1),
                        )
                        .changed()
                    {
                        *value = percent / 100.0 * animation.duration;
                    }
                    let mut frame = (*value / bvh.frame_time).round() as usize;
                    if ui
                        .add_sized(
                            Vec2::new(52.0, 26.0),
                            egui::DragValue::new(&mut frame).range(0..=bvh.frames.saturating_sub(2)),
                        )
                        .changed()
                    {
                        *value = frame as f32 * bvh.frame_time;
                    }
                    ui.end_row();
                }
            });
        animation.loop_out = animation.loop_out.max(animation.loop_in);
    });
    ui.add_space(12.0);
    egui::Grid::new("animation_ease")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 8.0))
        .show(ui, |ui| {
            for (label, value) in [
                (aurora_i18n::tr!("import-ease-in"), &mut animation.ease_in),
                (aurora_i18n::tr!("import-ease-out"), &mut animation.ease_out),
            ] {
                ui.label(label);
                ui.add(
                    egui::DragValue::new(value)
                        .range(0.0..=animation.duration)
                        .suffix(" s")
                        .speed(0.01)
                        .fixed_decimals(3),
                );
                ui.end_row();
            }
        });
}

fn map_label(slot: usize) -> &'static str {
    match slot {
        0 => aurora_i18n::tr!("import-base-color"),
        1 => aurora_i18n::tr!("import-normal"),
        2 => aurora_i18n::tr!("import-metallic-roughness"),
        _ => aurora_i18n::tr!("import-emissive"),
    }
}
fn camera_hint(ui: &mut egui::Ui, p: &Palette) {
    ui.label(RichText::new(aurora_i18n::tr!("import-preview-gestures")).color(p.muted));
}
fn preview_camera_controls(ui: &mut egui::Ui, p: &Palette, view: &mut aurora_render::preview::View) -> egui::Response {
    let response = action_button(
        ui,
        p,
        "refresh",
        aurora_i18n::tr!("import-preview-reset-view"),
        ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .on_hover_text(aurora_i18n::tr!("import-preview-camera-help"));
    if response.clicked() {
        view.reset_camera();
    }
    response
}
fn alpha_name(mode: aurora_assets::AlphaMode) -> &'static str {
    match mode {
        aurora_assets::AlphaMode::Opaque => aurora_i18n::tr!("import-alpha-opaque"),
        aurora_assets::AlphaMode::Blend => aurora_i18n::tr!("import-alpha-blend"),
        aurora_assets::AlphaMode::Mask => aurora_i18n::tr!("import-alpha-mask"),
    }
}
