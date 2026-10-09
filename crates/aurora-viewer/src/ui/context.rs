//! Right-click menu in the world (basic version of Firestorm's pie/context
//! menus for land, objects, other avatars and our own avatar).

use crate::theme::Palette;
use egui::{RichText, Vec2};
use glam::Vec3;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Target {
    Ground,
    Object {
        local_id: u32,
        full_id: Uuid,
        offset: Vec3,
        name: String,
    },
    /// `complexity`: (value, shown as a silhouette); `render`: exception
    /// 0 normal, 1 never fully, 2 always fully.
    Avatar {
        id: Uuid,
        name: String,
        own: bool,
        complexity: Option<(u32, bool)>,
        render: u8,
        blocked: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub pos: egui::Pos2,
    /// World point that was clicked.
    pub point: Vec3,
    pub target: Target,
}

#[derive(Debug, Clone, Copy)]
pub enum CtxAction {
    Touch(u32),
    Sit {
        target: Uuid,
        offset: Vec3,
    },
    Zoom(Vec3),
    /// About Land on the parcel at the clicked point (LLToolPie selects it).
    AboutLand(Vec3),
    StandUp,
    SitGround,
    ToggleFly,
    ResetCamera,
    Im(Uuid),
    OfferTeleport(Uuid),
    Profile(Uuid),
    /// Rendering exception of an avatar (LLRenderMuteList).
    SetRender(Uuid, u8),
    /// Block / unblock a resident (LLMuteList).
    ToggleBlock(Uuid),
    /// Block an object (LLMute::OBJECT).
    BlockObject(Uuid),
    /// Edit the object in the build tools (pie menu "Edit").
    Edit(Uuid),
    /// Take a copy into the inventory.
    TakeCopy(Uuid),
    /// Change our display name (LLFloaterDisplayName).
    DisplayName,
}

/// egui temp data: (rows drawn, hovered row) of the open menu, and the row
/// hovered last frame.
const ROWS: &str = "world_context_menu_rows";
const HOVERED: &str = "world_context_menu_hovered";

/// Flat menu row; disabled rows are greyed (SL shows unavailable actions).
fn item(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> bool {
    let w = ui.available_width().max(170.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), egui::Sense::click());
    // row number and hovered row, for the slice sounds
    ui.ctx().data_mut(|d| {
        let (n, hovered) = d.get_temp_mut_or_default::<(usize, Option<usize>)>(egui::Id::new(ROWS));
        if enabled && resp.hovered() {
            *hovered = Some(*n);
        }
        *n += 1;
    });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.violet.gamma_multiply(0.35));
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        if enabled { p.ink } else { p.muted_dim },
    );
    resp.clicked() && enabled
}

fn separator(ui: &mut egui::Ui, p: &Palette) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 7.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.raised));
}

/// Draw the open menu; returns the chosen action (the menu closes).
pub fn show(ctx: &egui::Context, p: &Palette, menu: &mut Option<ContextMenu>, seated: bool, flying: bool) -> Option<CtxAction> {
    let m = menu.as_ref()?.clone();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(ROWS), (0usize, None::<usize>)));
    let mut action = None;
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("world_context_menu"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .fixed_pos(m.pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.panel)
                .stroke(egui::Stroke::new(1.0, p.raised))
                .corner_radius(egui::CornerRadius::same(3))
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(190.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let title = match &m.target {
                        Target::Ground => "Terrain".to_owned(),
                        Target::Object { name, .. } => name.clone(),
                        Target::Avatar { name, .. } => name.clone(),
                    };
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(title).size(11.5).strong().color(p.violet_light));
                    });
                    if let Target::Avatar {
                        complexity: Some((c, silhouette)),
                        ..
                    } = &m.target
                    {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            let text = format!(
                                "Complexité : {}{}",
                                super::hud::group_digits(*c),
                                if *silhouette { " (silhouette)" } else { "" }
                            );
                            ui.label(RichText::new(text).size(10.5).color(p.muted));
                        });
                    }
                    separator(ui, p);
                    match &m.target {
                        Target::Object {
                            local_id, full_id, offset, ..
                        } => {
                            if item(ui, p, "Toucher", true) {
                                action = Some(CtxAction::Touch(*local_id));
                            }
                            if item(ui, p, "S'asseoir ici", true) {
                                action = Some(CtxAction::Sit {
                                    target: *full_id,
                                    offset: *offset,
                                });
                            }
                            if item(ui, p, "Zoomer", true) {
                                action = Some(CtxAction::Zoom(m.point));
                            }
                            separator(ui, p);
                            if item(ui, p, "Modifier", true) {
                                action = Some(CtxAction::Edit(*full_id));
                            }
                            if item(ui, p, "Prendre une copie", true) {
                                action = Some(CtxAction::TakeCopy(*full_id));
                            }
                            item(ui, p, "Payer", false);
                            if item(ui, p, "Bloquer", true) {
                                action = Some(CtxAction::BlockObject(*full_id));
                            }
                        }
                        Target::Avatar { id, own: false, .. } => {
                            if item(ui, p, "Profil", true) {
                                action = Some(CtxAction::Profile(*id));
                            }
                            if item(ui, p, "Envoyer un IM", true) {
                                action = Some(CtxAction::Im(*id));
                            }
                            if item(ui, p, "Proposer une téléportation", true) {
                                action = Some(CtxAction::OfferTeleport(*id));
                            }
                            if item(ui, p, "Zoomer", true) {
                                action = Some(CtxAction::Zoom(m.point));
                            }
                            // Firestorm "Render Normally / Do Not Render / Render Fully"
                            separator(ui, p);
                            if let Target::Avatar { render, .. } = &m.target {
                                for (mode, label) in [
                                    (0u8, "Affichage normal"),
                                    (2, "Toujours afficher en entier"),
                                    (1, "Ne jamais afficher (silhouette)"),
                                ] {
                                    let mark = if *render == mode { "•  " } else { "    " };
                                    if item(ui, p, &format!("{mark}{label}"), true) {
                                        action = Some(CtxAction::SetRender(*id, mode));
                                    }
                                }
                            }
                            separator(ui, p);
                            item(ui, p, "Ajouter en ami", false);
                            item(ui, p, "Payer", false);
                            if let Target::Avatar { blocked, .. } = &m.target
                                && item(ui, p, if *blocked { "Débloquer" } else { "Bloquer" }, true)
                            {
                                action = Some(CtxAction::ToggleBlock(*id));
                            }
                        }
                        Target::Avatar { own: true, .. } => {
                            if seated {
                                if item(ui, p, "Se lever", true) {
                                    action = Some(CtxAction::StandUp);
                                }
                            } else if item(ui, p, "S'asseoir par terre", true) {
                                action = Some(CtxAction::SitGround);
                            }
                            if item(ui, p, if flying { "Atterrir" } else { "Voler" }, true) {
                                action = Some(CtxAction::ToggleFly);
                            }
                            if item(ui, p, "Réinitialiser la caméra", true) {
                                action = Some(CtxAction::ResetCamera);
                            }
                            separator(ui, p);
                            if let Target::Avatar { id, .. } = &m.target
                                && item(ui, p, "Mon profil", true)
                            {
                                action = Some(CtxAction::Profile(*id));
                            }
                            item(ui, p, "Apparence", false);
                            if item(ui, p, "Nom d'affichage…", true) {
                                action = Some(CtxAction::DisplayName);
                            }
                        }
                        Target::Ground => {
                            if seated && item(ui, p, "Se lever", true) {
                                action = Some(CtxAction::StandUp);
                            }
                            if item(ui, p, "Zoomer", true) {
                                action = Some(CtxAction::Zoom(m.point));
                            }
                            if item(ui, p, "À propos du terrain", true) {
                                action = Some(CtxAction::AboutLand(m.point));
                            }
                            separator(ui, p);
                            item(ui, p, "Construire", false);
                        }
                    }
                    ui.add_space(2.0);
                });
        });
    // UISndPieMenuSliceHighlight0..7 when an enabled row gets hovered (the
    // eight slices of Firestorm's default pie menu, PieMenu::draw)
    let hovered = ctx
        .data(|d| d.get_temp::<(usize, Option<usize>)>(egui::Id::new(ROWS)))
        .and_then(|r| r.1);
    let before = ctx.data_mut(|d| d.get_temp::<Option<usize>>(egui::Id::new(HOVERED)).flatten());
    if hovered != before {
        if let Some(i) = hovered.filter(|i| *i < 8) {
            super::sound_cues::request(ctx, crate::ui_sound::UiSound::PieMenuSlice(i as u8));
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(HOVERED), hovered));
    }
    // click elsewhere or Escape closes it
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if action.is_some() || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if close {
        *menu = None;
        ctx.data_mut(|d| d.remove::<Option<usize>>(egui::Id::new(HOVERED)));
    }
    action
}
