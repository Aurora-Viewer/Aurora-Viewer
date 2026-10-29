//! Secondary floaters of the build floater: grid options
//! (floater_build_options.xml), "Advanced" weights (LLFloaterObjectWeights),
//! media settings (LLFloaterMediaSettings and its three panels, the
//! whitelist entry dialog), the group picker (LLFloaterGroupPicker), the
//! contents permissions (LLFloaterBulkPermission), an item's properties,
//! the texture picker and the confirmations (notifications.xml).

use super::common::{check, label, row, section, small, spin, spin_i, spin_with_min_decimals, text};
use super::{Confirm, Env};
use crate::build::{BuildSettings, BuildTool};
use crate::media::entry::MediaEntry;
use crate::theme::Palette;
use crate::ui::widgets::{Floater, flat_button, tabs};
use crate::world::World;
use aurora_net::build::BuildCmd;
use egui::{RichText, Vec2};

/// The media settings being edited (LLFloaterMediaSettings).
#[derive(Debug, Clone)]
pub struct MediaSettingsState {
    pub entry: MediaEntry,
    pub tab: usize,
    whitelist_sel: Option<usize>,
    /// The "Whitelist entry" dialog, open with its text.
    whitelist_entry: Option<String>,
}

impl MediaSettingsState {
    /// From the first selected face's entry; a new media autoplays and
    /// scales (EXT-5172 defaults of the panel).
    pub fn new(entry: Option<MediaEntry>) -> MediaSettingsState {
        MediaSettingsState {
            entry: entry.unwrap_or(MediaEntry {
                auto_play: true,
                auto_scale: true,
                width_pixels: 256,
                height_pixels: 256,
                ..Default::default()
            }),
            tab: 0,
            whitelist_sel: None,
            whitelist_entry: None,
        }
    }
}

pub fn secondary(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, _s: &mut BuildSettings, env: &mut Env) {
    weights(ctx, p, tool, world);
    media_settings(ctx, p, tool, world, env);
    group_picker(ctx, p, tool, world, "group_picker");
    super::contents_permissions::show(ctx, p, tool, world);
    super::item_properties::show(ctx, p, tool, world, env);
    confirm(ctx, p, tool, world, env);
    super::texture_preview::show(ctx, p, tool, world, env);
    // the picker's previews are wanted images too
    let w: Vec<uuid::Uuid> = tool.ui.picker.wanted_images.drain().collect();
    tool.ui.wanted_images.extend(w);
}

/// floater_build_options.xml, shared by the inline construction settings.
pub(super) fn ruler_controls(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    let label_width = (ui.available_width() - 80.0).max(30.0);
    ui.spacing_mut().slider_width = (ui.available_width() - 80.0).clamp(40.0, 170.0);
    row(ui, p, aurora_i18n::tr!("build-ui-floaters-unit-meters"), label_width, |ui| {
        spin(ui, &mut s.grid_resolution, 0.01, 0.01..=5.0, 3, 70.0, false);
    });
    row(
        ui,
        p,
        aurora_i18n::tr!("build-ui-floaters-intersections-up-to-meters"),
        label_width,
        |ui| {
            spin(ui, &mut s.grid_draw_size, 0.5, 1.0..=50.0, 1, 70.0, false);
        },
    );
    row(ui, p, aurora_i18n::tr!("build-ui-floaters-decimal-precision"), label_width, |ui| {
        let mut d = s.decimal_precision as i32;
        if spin_i(ui, &mut d, 0..=7, 70.0).changed {
            s.decimal_precision = d as u8;
        }
    });
    row(ui, p, aurora_i18n::tr!("build-ui-floaters-angle-degrees"), label_width, |ui| {
        spin(ui, &mut s.rotation_step, 0.1, 0.01..=359.99, 2, 70.0, false);
    });
    if let Some(v) = check(
        ui,
        p,
        s.grid_sub_unit,
        false,
        aurora_i18n::tr!("build-ui-floaters-show-intermediate-tick-marks"),
        true,
    ) {
        s.grid_sub_unit = v;
    }
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("build-ui-floaters-opacity"));
        ui.add(
            egui::Slider::new(&mut s.grid_opacity, 0.0..=1.0)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.05),
        );
    });
}

/// Aurora: the size of the edit handles, in percent of the Legacy Viewer's
/// fixed screen sizes (#1328).
pub(super) fn handle_controls(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    ui.spacing_mut().slider_width = (ui.available_width() - 80.0).clamp(40.0, 170.0);
    let range = crate::build::GIZMO_SCALE_RANGE;
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("build-ui-floaters-handle-size"));
        let mut percent = (s.gizmo_scale() * 100.0).round();
        if ui
            .add(
                egui::Slider::new(&mut percent, range.start() * 100.0..=range.end() * 100.0)
                    .custom_parser(aurora_i18n::parse_number)
                    .step_by(10.0),
            )
            .changed()
        {
            s.gizmo_scale = percent / 100.0;
        }
    });
}

/// The Build 1 preferences' "Limit select distance" and "Limit drag
/// distance" (panel_preferences_firestorm.xml): a switch and its distance in
/// metres, greyed while the switch is off.
pub(super) fn distance_controls(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    let label_width = (ui.available_width() - 80.0).max(30.0);
    for (limit, distance, text) in [
        (
            &mut s.limit_select_distance,
            &mut s.max_select_distance,
            aurora_i18n::tr!("build-ui-floaters-limit-select-distance"),
        ),
        (
            &mut s.limit_drag_distance,
            &mut s.max_drag_distance,
            aurora_i18n::tr!("build-ui-floaters-limit-drag-distance"),
        ),
    ] {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(label_width, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(label_width);
                    if let Some(v) = check(ui, p, *limit, false, text, true) {
                        *limit = v;
                    }
                },
            );
            ui.add_enabled_ui(*limit, |ui| {
                spin_with_min_decimals(ui, distance, 1.0, crate::build::DISTANCE_LIMIT_RANGE, 3, 70.0, false, 0);
            });
        });
    }
}

/// LLFloaterObjectWeights ("Advanced").
fn weights(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &World) {
    if !tool.ui.weights_open {
        return;
    }
    let mut open = true;
    let pos = ctx.content_rect().left_top() + Vec2::new(352.0, 290.0);
    Floater::new("object_weights", aurora_i18n::tr!("common-advanced"), pos, Vec2::new(210.0, 360.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let none = tool.selection.is_empty();
            let v = |x: Option<String>| if none { "--".to_owned() } else { x.unwrap_or_else(|| "…".into()) };
            let line = |ui: &mut egui::Ui, value: String, name: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized([60.0, 16.0], egui::Label::new(RichText::new(value).size(12.0).color(p.ink)));
                    label(ui, p, name);
                });
            };
            section(ui, p, aurora_i18n::tr!("build-ui-floaters-selected"));
            line(ui, v(Some(tool.roots(world).len().to_string())), aurora_i18n::tr!("common-objects"));
            line(
                ui,
                v(Some(tool.sel_prims(world).len().to_string())),
                aurora_i18n::tr!("build-ui-floaters-prims"),
            );
            section(ui, p, aurora_i18n::tr!("build-ui-floaters-selection-weights"));
            let w = match &tool.costs.weights {
                Some(Ok(w)) => Some(*w),
                Some(Err(_)) => None,
                None => None,
            };
            let failed = matches!(tool.costs.weights, Some(Err(_)));
            let f = |x: Option<f32>| {
                if failed {
                    Some("--".into())
                } else {
                    x.map(|x| aurora_i18n::number((x) as f64, 1))
                }
            };
            line(ui, v(f(w.map(|w| w.download))), aurora_i18n::tr!("build-ui-floaters-download"));
            line(ui, v(f(w.map(|w| w.physics))), aurora_i18n::tr!("build-ui-floaters-physical"));
            line(ui, v(f(w.map(|w| w.server))), aurora_i18n::tr!("build-ui-floaters-server"));
            line(ui, v(Some(tool.render_cost(world).to_string())), aurora_i18n::tr!("common-display"));
            section(ui, p, aurora_i18n::tr!("build-ui-floaters-land-impact"));
            let cap = tool.costs.capacity(world);
            let rezzed = tool.costs.parcel.as_ref().map(|(_, pi)| pi.sim_total_prims);
            line(
                ui,
                v(tool.land_impact(world).map(|x| (x as i32).to_string())),
                aurora_i18n::tr!("build-ui-floaters-selected-2"),
            );
            line(
                ui,
                v(rezzed.map(|x| x.to_string())),
                aurora_i18n::tr!("build-ui-floaters-rezzed-on-land"),
            );
            line(
                ui,
                v(cap.map(|c| c.1.to_string())),
                aurora_i18n::tr!("build-ui-floaters-remaining-capacity"),
            );
            line(
                ui,
                v(cap.map(|c| c.0.to_string())),
                aurora_i18n::tr!("build-ui-floaters-total-capacity"),
            );
            section(ui, p, aurora_i18n::tr!("extra-rendering-info"));
            let (tris, area) = tool.render_info(world);
            line(ui, v(Some(tris.to_string())), aurora_i18n::tr!("build-ui-floaters-triangles-shown"));
            line(
                ui,
                v(Some(aurora_i18n::number((area) as f64, 0))),
                aurora_i18n::tr!("build-ui-floaters-pixel-area"),
            );
        });
    tool.ui.weights_open = open;
}

/// LLFloaterMediaSettings: General, Customize, Security; OK / Cancel / Apply.
fn media_settings(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let Some(mut st) = tool.ui.media_settings.take() else { return };
    let mut open = true;
    let mut apply = false;
    let mut close = false;
    let pos = ctx.content_rect().center() - Vec2::new(200.0, 220.0);
    Floater::new(
        "media_settings",
        aurora_i18n::tr!("build-ui-floaters-media-settings"),
        pos,
        Vec2::new(400.0, 440.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        tabs(
            ui,
            p,
            &mut st.tab,
            &[
                (aurora_i18n::tr!("common-general"), true),
                (aurora_i18n::tr!("common-customize"), true),
                (aurora_i18n::tr!("build-ui-floaters-security"), true),
            ],
        );
        ui.add_space(6.0);
        let e = &mut st.entry;
        match st.tab {
            0 => {
                label(ui, p, aurora_i18n::tr!("common-home-page"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.home_url)
                        .char_limit(1024)
                        .desired_width(f32::INFINITY),
                );
                if !e.home_url.is_empty() && !e.check_url(&e.home_url) {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("build-ui-floaters-this-page-does-not-meet-the-whitelist-criteria"))
                            .size(11.0)
                            .color(p.warn),
                    );
                }
                let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 110.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    aurora_i18n::tr!("build-ui-floaters-preview"),
                    egui::FontId::proportional(12.0),
                    p.muted,
                );
                ui.horizontal(|ui| {
                    label(ui, p, aurora_i18n::tr!("build-ui-floaters-current-page"));
                    text(ui, p, &e.current_url);
                });
                if ui
                    .button(RichText::new(aurora_i18n::tr!("common-reset")).size(12.0))
                    .on_hover_text(aurora_i18n::tr!("build-ui-floaters-back-to-home-page"))
                    .clicked()
                {
                    tool.media_home(world, env, &e.home_url);
                }
                if let Some(v) = check(ui, p, e.auto_loop, false, aurora_i18n::tr!("build-ui-floaters-loop-playback"), true) {
                    e.auto_loop = v;
                }
                if let Some(v) = check(
                    ui,
                    p,
                    e.first_click_interact,
                    false,
                    aurora_i18n::tr!("build-ui-floaters-interact-on-first-click"),
                    true,
                ) {
                    e.first_click_interact = v;
                }
                if let Some(v) = check(
                    ui,
                    p,
                    e.auto_zoom,
                    false,
                    aurora_i18n::tr!("build-ui-floaters-automatic-zoom"),
                    true,
                ) {
                    e.auto_zoom = v;
                }
                if let Some(v) = check(
                    ui,
                    p,
                    e.auto_play,
                    false,
                    aurora_i18n::tr!("build-ui-floaters-auto-play-media"),
                    true,
                ) {
                    e.auto_play = v;
                }
                small(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-floaters-note-residents-may-override-this-setting"),
                );
                if let Some(v) = check(
                    ui,
                    p,
                    e.auto_scale,
                    false,
                    aurora_i18n::tr!("build-ui-floaters-automatically-resize-media-to-the-object-face"),
                    true,
                ) {
                    e.auto_scale = v;
                }
                ui.horizontal(|ui| {
                    label(ui, p, aurora_i18n::tr!("common-size"));
                    ui.add_enabled_ui(!e.auto_scale, |ui| {
                        let mut w = e.width_pixels as i32;
                        let mut h = e.height_pixels as i32;
                        spin_i(ui, &mut w, 0..=2048, 60.0);
                        ui.label("X");
                        spin_i(ui, &mut h, 0..=2048, 60.0);
                        e.width_pixels = w as u16;
                        e.height_pixels = h as u16;
                    });
                });
            }
            1 => {
                let controls = [
                    (aurora_i18n::tr!("build-ui-floaters-standard"), 0u8),
                    (aurora_i18n::tr!("build-ui-floaters-mini"), 1),
                ];
                ui.horizontal(|ui| {
                    label(ui, p, aurora_i18n::tr!("build-ui-floaters-controls"));
                    if let Some(c) = super::common::combo(ui, "media_controls", Some(e.controls), &controls, 100.0, true) {
                        e.controls = c;
                    }
                });
                let group_name = tool
                    .props
                    .values()
                    .next()
                    .and_then(|pr| world.groups.group(&pr.group_id))
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                for (title, bit) in [
                    (aurora_i18n::tr!("common-owner").to_owned(), crate::media::entry::PERM_OWNER),
                    (
                        aurora_i18n::tr!("extra-group", group_name = group_name.to_string()),
                        crate::media::entry::PERM_GROUP,
                    ),
                    (
                        aurora_i18n::tr!("build-ui-floaters-anyone").to_owned(),
                        crate::media::entry::PERM_ANYONE,
                    ),
                ] {
                    section(ui, p, &title);
                    if let Some(v) = check(
                        ui,
                        p,
                        e.perms_interact & bit != 0,
                        false,
                        aurora_i18n::tr!("build-ui-floaters-allow-navigation-interaction"),
                        true,
                    ) {
                        e.perms_interact = if v { e.perms_interact | bit } else { e.perms_interact & !bit };
                    }
                    if let Some(v) = check(
                        ui,
                        p,
                        e.perms_control & bit != 0,
                        false,
                        aurora_i18n::tr!("build-ui-floaters-show-control-bar"),
                        true,
                    ) {
                        e.perms_control = if v { e.perms_control | bit } else { e.perms_control & !bit };
                    }
                }
            }
            _ => {
                if let Some(v) = check(
                    ui,
                    p,
                    e.whitelist_enable,
                    false,
                    aurora_i18n::tr!("build-ui-floaters-allow-access-only-to-specified-urls"),
                    true,
                ) {
                    e.whitelist_enable = v;
                }
                egui::Frame::new().fill(p.field).inner_margin(4).show(ui, |ui| {
                    ui.set_min_size(Vec2::new(370.0, 200.0));
                    for (i, w) in e.whitelist.iter().enumerate() {
                        let rejected = !e.home_url.is_empty() && {
                            let probe = MediaEntry {
                                whitelist_enable: true,
                                whitelist: vec![w.clone()],
                                ..Default::default()
                            };
                            !probe.check_url(&e.home_url)
                        };
                        let mut t = RichText::new(w).size(12.0);
                        if rejected {
                            t = t.color(p.danger);
                        }
                        if ui.selectable_label(st.whitelist_sel == Some(i), t).clicked() {
                            st.whitelist_sel = Some(i);
                        }
                    }
                });
                small(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-floaters-entries-with-rejected-home-pages-are-shown-in-red"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            e.whitelist.len() < 64,
                            egui::Button::new(RichText::new(aurora_i18n::tr!("common-add")).size(12.0)),
                        )
                        .clicked()
                    {
                        st.whitelist_entry = Some(String::new());
                    }
                    if ui
                        .add_enabled(
                            st.whitelist_sel.is_some(),
                            egui::Button::new(RichText::new(aurora_i18n::tr!("common-delete")).size(12.0)),
                        )
                        .clicked()
                        && let Some(i) = st.whitelist_sel.take()
                        && i < e.whitelist.len()
                    {
                        e.whitelist.remove(i);
                    }
                });
            }
        }
        crate::ui::widgets::divider(ui, p);
        ui.horizontal(|ui| {
            if flat_button(ui, p, aurora_i18n::tr!("common-ok")).clicked() {
                apply = true;
                close = true;
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                close = true;
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-apply")).clicked() {
                apply = true;
            }
        });
    });
    // whitelist entry dialog (floater_whitelist_entry.xml)
    if let Some(mut entry) = st.whitelist_entry.take() {
        let mut keep = true;
        crate::ui::widgets::Modal::new(egui::Id::unique("whitelist_entry")).show(ctx, |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("build-ui-floaters-whitelist-entry"))
                    .size(13.0)
                    .color(p.ink),
            );
            small(ui, p, aurora_i18n::tr!("build-ui-floaters-enter-a-url-or-url-pattern-to-add-to"));
            ui.add(egui::TextEdit::singleline(&mut entry).desired_width(300.0));
            ui.horizontal(|ui| {
                if crate::ui::widgets::default_button(flat_button(ui, p, aurora_i18n::tr!("common-ok"))).clicked() {
                    let t = entry.trim();
                    if !t.is_empty() && st.entry.whitelist.len() < 64 {
                        st.entry.whitelist.push(t.to_owned());
                    }
                    keep = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    keep = false;
                }
            });
        });
        if keep {
            st.whitelist_entry = Some(entry);
        }
    }
    if apply {
        // home_url is required for a new media (selectionSetMedia)
        let entry = (!st.entry.home_url.trim().is_empty()).then(|| st.entry.clone());
        if entry.is_some() {
            tool.apply_media(world, env, entry);
        }
    }
    if open && !close {
        tool.ui.media_settings = Some(st);
    }
}

/// LLFloaterGroupPicker: the agent's groups and "none".
pub(super) fn group_picker(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, window: &'static str) {
    if !tool.ui.group_picker_open {
        return;
    }
    let mut open = true;
    let mut chosen = None;
    let pos = ctx.content_rect().center() - Vec2::new(130.0, 150.0);
    Floater::new(
        window,
        aurora_i18n::tr!("build-ui-floaters-choose-a-group"),
        pos,
        Vec2::new(260.0, 300.0),
    )
    .show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
            if ui
                .selectable_label(false, RichText::new(aurora_i18n::tr!("extra-none")).size(12.0))
                .double_clicked()
            {
                chosen = Some(uuid::Uuid::nil());
            }
            let mut groups: Vec<(uuid::Uuid, String)> = world.groups.groups.iter().map(|g| (g.id, g.name.clone())).collect();
            groups.sort_by_key(|g| g.1.to_lowercase());
            for (id, name) in groups {
                if ui.selectable_label(false, RichText::new(name).size(12.0)).double_clicked() {
                    chosen = Some(id);
                }
            }
        });
        small(ui, p, aurora_i18n::tr!("build-ui-floaters-double-click-a-group"));
    });
    if let Some(g) = chosen {
        if super::general::can_set_group(tool, world) {
            tool.set_group(world, g);
        }
        open = false;
    }
    tool.ui.group_picker_open = open;
}

pub(super) fn confirm_answer(ctx: &egui::Context, p: &Palette, c: &Confirm, id: egui::Id) -> Option<bool> {
    let (title, body, yes) = match c {
        Confirm::Deed(_) => (
            aurora_i18n::tr!("build-ui-floaters-deed-object"),
            aurora_i18n::tr!("build-ui-floaters-if-you-deed-this-object-the-group-will-receive"),
            aurora_i18n::tr!("common-deed"),
        ),
        Confirm::Divide => (
            aurora_i18n::tr!("common-subdivide"),
            aurora_i18n::tr!("build-ui-floaters-divide-land-the-selection-will-become-a-new-parcel"),
            aurora_i18n::tr!("build-ui-floaters-divide"),
        ),
        Confirm::Join => (
            aurora_i18n::tr!("common-join"),
            aurora_i18n::tr!("build-ui-floaters-join-the-selected-parcels-into-one"),
            aurora_i18n::tr!("common-join"),
        ),
        Confirm::Release => (
            aurora_i18n::tr!("common-abandon-land"),
            aurora_i18n::tr!("build-ui-floaters-abandon-this-parcel-it-will-return-to-the-region"),
            aurora_i18n::tr!("build-ui-floaters-abandon"),
        ),
        Confirm::DeleteMedia => (
            aurora_i18n::tr!("build-ui-floaters-remove-media"),
            aurora_i18n::tr!("build-ui-floaters-remove-media-from-selected-faces"),
            aurora_i18n::tr!("common-delete"),
        ),
        Confirm::HideWater => (
            aurora_i18n::tr!("build-ui-floaters-occlude-water"),
            aurora_i18n::tr!("build-ui-floaters-checking-hide-water-overrides-texture-bumpiness-and-shininess-choices"),
            aurora_i18n::tr!("build-ui-floaters-continue"),
        ),
        Confirm::Probe => (
            aurora_i18n::tr!("common-reflection-probe"),
            aurora_i18n::tr!("build-ui-floaters-this-prim-will-become-a-reflection-probe-it-will"),
            aurora_i18n::tr!("build-ui-floaters-continue"),
        ),
    };
    let mut answer = None;
    crate::ui::widgets::Modal::new(id).show(ctx, |ui| {
        ui.set_max_width(320.0);
        ui.label(RichText::new(title).size(14.0).strong().color(p.ink));
        ui.add_space(4.0);
        ui.label(RichText::new(body).size(12.0).color(p.ink));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            use crate::ui::widgets::{ActionKind, action_button};
            let size = Vec2::new(0.0, 26.0);
            if action_button(ui, p, "check", yes, ActionKind::Primary, size).clicked() {
                answer = Some(true);
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
        });
    });
    answer
}

/// The confirmations, as modal dialogs.
fn confirm(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let Some(c) = tool.ui.confirm.clone() else { return };
    let Some(ok) = confirm_answer(ctx, p, &c, egui::Id::unique("build_confirm")) else {
        return;
    };
    tool.ui.confirm = None;
    if !ok {
        return;
    }
    match c {
        Confirm::Deed(group) => {
            if super::general::can_deed(tool, world, group) {
                tool.deed(world, group);
            }
        }
        Confirm::Divide | Confirm::Join => {
            if let Some((handle, west, south, east, north)) = tool.land.selection_local(world) {
                tool.send(if c == Confirm::Divide {
                    BuildCmd::ParcelDivide {
                        handle,
                        west,
                        south,
                        east,
                        north,
                    }
                } else {
                    BuildCmd::ParcelJoin {
                        handle,
                        west,
                        south,
                        east,
                        north,
                    }
                });
            }
        }
        Confirm::Release => {
            let target = tool.land.parcel.clone().or_else(|| {
                let h = world.regions.values().find(|r| r.info.as_ref().is_some_and(|i| i.is_main))?.handle;
                world.parcel.clone().map(|pi| (h, pi))
            });
            if let Some((handle, pi)) = target {
                tool.send(BuildCmd::ParcelRelease {
                    handle,
                    local_id: pi.local_id,
                });
                tool.land.selection = None;
            }
        }
        Confirm::DeleteMedia => tool.apply_media(world, env, None),
        Confirm::HideWater => {
            // FSPanelFace::onCommitHideWater
            let sel: Vec<(usize, u8)> = tool.sel_te(world);
            tool.edit_faces(world, |f, _, _| {
                f.texture = aurora_prim::te::WATER_EXCLUSION_TEXTURE;
                f.bump_shiny_fullbright &= 0x20;
                f.color[3] = 1.0;
            });
            let puts = sel.into_iter().map(|(i, f)| (i, f, None)).collect();
            tool.legacy_put(world, puts);
        }
        Confirm::Probe => {
            if let Some(idx) = tool.selection.first().and_then(|k| world.objects.index_of(k)) {
                tool.make_probe(world, idx);
                if !tool.ui.probe_warned {
                    tool.status =
                        aurora_i18n::tr!("build-ui-floaters-reflection-probes-can-only-be-selected-when-select-reflection").into();
                    tool.ui.probe_warned = true;
                }
            }
        }
    }
}

impl BuildTool {
    /// "Reset" of the media settings: the selected faces' media
    /// go back to the home page (LLPanelMediaSettingsGeneral::onBtnResetCurrentUrl).
    fn media_home(&mut self, world: &World, env: &mut Env, home: &str) {
        for (idx, face) in self.sel_te(world) {
            let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            if let Some(m) = env.media.find_mut(&crate::media::MediaKey::Prim { object: id, face }) {
                m.navigate(home);
            }
        }
    }

    /// "Display" of the weights (LLSelectMgr::getSelectedObjectRenderCost,
    /// coarse: triangles of the selection's volumes / 10, textures 1 each).
    pub fn render_cost(&self, world: &World) -> u32 {
        let (tris, _) = self.render_info(world);
        let textures: std::collections::HashSet<uuid::Uuid> = self
            .sel_prims(world)
            .into_iter()
            .filter_map(|i| world.objects.get(i)?.te.clone())
            .flat_map(|te| te.faces.iter().map(|f| f.texture).collect::<Vec<_>>())
            .collect();
        (tris / 10) as u32 + textures.len() as u32
    }

    /// Rendering info of the weights floater: triangles and screen area.
    pub fn render_info(&self, world: &World) -> (usize, f32) {
        let mut tris = 0;
        let mut area = 0.0;
        for i in self.sel_prims(world) {
            let Some(o) = world.objects.get(i) else { continue };
            if !o.volume.is_mesh() {
                tris += aurora_prim::volume::num_faces(&o.volume) * 2 * 16;
            }
            if let Some((pos, _, _)) = crate::scene::Scene::object_transform(world, i, std::time::Instant::now(), 0) {
                let d = pos.distance(self.cam.eye).max(0.1);
                let r = o.scale.length() * 0.5 * self.cam.pixel_meter_ratio() / d;
                area += std::f32::consts::PI * r * r;
            }
        }
        (tris, area)
    }
}
