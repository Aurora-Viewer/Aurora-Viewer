//! "Script info" of About Land: the script memory and URLs of the parcels
//! of the selected parcel's owner in the region, object by object
//! (LLFloaterScriptLimits, llfloaterscriptlimits.cpp: LandResources, then
//! ScriptResourceSummary and ScriptResourceDetails; Refresh, Highlight as a
//! beacon, Return), and those of my attachments, which Firestorm shows in
//! its own floater (LLFloaterMyScripts, llfloatermyscripts.cpp:
//! AttachmentResources).

use super::View;
use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use aurora_net::land::{ScriptObject, ScriptSummary};
use egui::{RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// SIZE_OF_ONE_KB.
const KB: i64 = 1024;

#[derive(Default)]
pub(super) struct ScriptsUi {
    pub(super) open: bool,
    /// 0 the parcel, 1 my attachments.
    tab: usize,
    /// The objects ticked in the parcel's list.
    selected: HashSet<Uuid>,
    /// The request chain waits for the parcel UUID (RemoteParcelRequest).
    pending: bool,
    mine_asked: bool,
}

impl ScriptsUi {
    /// Open on the parcel's tab and ask (StartRequestChain).
    pub(super) fn open(&mut self) {
        self.open = true;
        self.tab = 0;
        self.pending = true;
        self.mine_asked = false;
        self.selected.clear();
    }
}

/// "Memory used: 1 234 KB of 8 192 KB, 6 958 KB available" (or without a
/// maximum), then the URLs: ScriptLimitsMemoryUsed[Simple], ScriptLimitsURLsUsed.
pub(super) fn summary_lines(s: &ScriptSummary) -> Vec<String> {
    let int = |v: i64| aurora_i18n::integer(v);
    let mut out = Vec::new();
    if let Some((used, max)) = s.memory {
        let (used, max) = (used / KB, max / KB);
        out.push(if max > 0 {
            aurora_i18n::tr!(
                "ui-land-scripts-memory",
                used = int(used),
                max = int(max),
                available = int(max - used)
            )
        } else {
            aurora_i18n::tr!("ui-land-scripts-memory-simple", used = int(used))
        });
    }
    if let Some((used, max)) = s.urls {
        out.push(aurora_i18n::tr!(
            "ui-land-scripts-urls",
            used = int(used),
            max = int(max),
            available = int(max - used)
        ));
    }
    out
}

/// "<140, 130, 25>" like the list's location column.
pub(super) fn location_text(o: &ScriptObject) -> String {
    o.location
        .map_or_else(String::new, |l| format!("<{:.0}, {:.0}, {:.0}>", l.x, l.y, l.z))
}

/// Why the parcel's tab cannot ask (StartRequestChain), None when it can.
fn not_available(v: &View, world: &World) -> Option<&'static str> {
    if v.parcel.is_none() {
        return Some(aurora_i18n::tr!("ui-land-scripts-no-parcel"));
    }
    if !v.same_region {
        return Some(aurora_i18n::tr!("ui-land-scripts-wrong-region"));
    }
    match world.land.sel.as_ref().and_then(|s| s.parcel_uuid) {
        Some(None) => Some(aurora_i18n::tr!("ui-land-scripts-error")),
        _ => None,
    }
}

/// Columns of a list: (title, width, right aligned); a zero width shares
/// the room left.
fn head(ui: &mut egui::Ui, p: &Palette, cols: &[(&str, f32, bool)]) -> Vec<f32> {
    let flex = cols.iter().filter(|c| c.1 == 0.0).count().max(1) as f32;
    let fixed: f32 = cols.iter().map(|c| c.1).sum();
    let share = ((ui.available_width() - fixed) / flex).max(60.0);
    let widths: Vec<f32> = cols.iter().map(|c| if c.1 == 0.0 { share } else { c.1 }).collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for ((title, _, right), w) in cols.iter().zip(&widths) {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(*w, 20.0), Sense::hover());
            let g = ui.painter().layout_job(super::aurora::semibold(ui, title, 11.0, p.muted_dim));
            let x = if *right {
                rect.right() - 8.0 - g.size().x
            } else {
                rect.left() + 6.0
            };
            ui.painter()
                .galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.muted_dim);
        }
    });
    widths
}

/// One row: its cells, right aligned where asked; true when clicked.
fn row(ui: &mut egui::Ui, p: &Palette, widths: &[f32], cells: &[(String, bool)], selected: bool) -> egui::Response {
    let width: f32 = widths.iter().sum();
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
    } else if r.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.4));
    }
    let mut x = rect.left();
    for ((text, right), w) in cells.iter().zip(widths) {
        let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), egui::FontId::proportional(12.0), p.ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width(w - 12.0);
        let g = ui.painter().layout_job(job);
        let gx = if *right { x + w - 8.0 - g.size().x } else { x + 6.0 };
        ui.painter().galley(egui::pos2(gx, rect.center().y - g.size().y / 2.0), g, p.ink);
        x += w;
    }
    r
}

fn status(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(12.0).color(p.muted));
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, st: &mut ScriptsUi, v: &View, world: &mut World) {
    if !st.open {
        return;
    }
    // the request chain: the parcel UUID first (RemoteParcelRequest); a
    // parcel still on its way is waited for
    if st.pending {
        match not_available(v, world) {
            None => {
                if let Some(Some(id)) = world.land.sel.as_ref().and_then(|s| s.parcel_uuid) {
                    st.pending = false;
                    world.land.request_scripts(id);
                }
            }
            Some(_) if v.parcel.is_some() => st.pending = false,
            Some(_) => {}
        }
    }
    if st.tab == 1 && !st.mine_asked {
        st.mine_asked = true;
        world.land.request_attachment_scripts();
    }
    let screen = ctx.content_rect();
    let size = Vec2::new(680.0, 440.0);
    let mut open = st.open;
    let mut floater = Floater::new(
        "land_scripts",
        aurora_i18n::tr!("ui-land-general-script-info"),
        screen.center() - size / 2.0,
        size,
    );
    floater.min_size = Vec2::new(520.0, 300.0);
    floater.show(ctx, p, &mut open, |ui| {
        widgets::tabs(
            ui,
            p,
            &mut st.tab,
            &[
                (aurora_i18n::tr!("ui-land-scripts-parcel-tab"), true),
                (aurora_i18n::tr!("ui-land-scripts-mine-tab"), true),
            ],
        );
        ui.add_space(6.0);
        if st.tab == 0 {
            parcel_tab(ui, p, st, v, world);
        } else {
            mine_tab(ui, p, st, world);
        }
    });
    st.open = open;
}

fn parcel_tab(ui: &mut egui::Ui, p: &Palette, st: &mut ScriptsUi, v: &View, world: &mut World) {
    let info = world.land.scripts.clone();
    let reason = not_available(v, world);
    let parcels = match &info.details {
        Some(Ok(list)) => list.clone(),
        _ => Vec::new(),
    };
    let objects: Vec<(i32, Option<i32>, String, ScriptObject)> = parcels
        .iter()
        .flat_map(|pa| {
            pa.objects
                .iter()
                .map(|o| (pa.local_id.unwrap_or(0), pa.local_id, pa.name.clone(), o.clone()))
        })
        .collect();
    st.selected.retain(|id| objects.iter().any(|o| o.3.id == *id));
    let has_locations = objects.iter().any(|o| o.3.location.is_some());
    let has_local_ids = parcels.iter().any(|pa| pa.local_id.is_some());
    // summary
    match (reason, &info.summary) {
        (Some(why), _) => status(ui, p, why),
        (None, None) => status(ui, p, aurora_i18n::tr!("ui-land-scripts-waiting")),
        (None, Some(Err(e))) => status(ui, p, &format!("{} ({e})", aurora_i18n::tr!("ui-land-scripts-error"))),
        (None, Some(Ok(s))) => {
            for line in summary_lines(s) {
                ui.label(RichText::new(line).size(12.5).color(p.ink));
            }
        }
    }
    if let Some(Ok(_)) = &info.details {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-scripts-parcels-listed", count = parcels.len() as i64))
                .size(12.0)
                .color(p.muted),
        );
    }
    ui.add_space(4.0);
    let widths = head(
        ui,
        p,
        &[
            (aurora_i18n::tr!("ui-land-scripts-size"), 80.0, true),
            (aurora_i18n::tr!("ui-land-scripts-urls-head"), 50.0, true),
            (aurora_i18n::tr!("common-name"), 0.0, false),
            (aurora_i18n::tr!("common-owner"), 0.0, false),
            (aurora_i18n::tr!("ui-land-scripts-parcel"), 0.0, false),
            (aurora_i18n::tr!("ui-land-scripts-location"), 110.0, false),
        ],
    );
    let list_h = (ui.available_height() - 40.0).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("land_scripts_parcel")
        .max_height(list_h)
        .min_scrolled_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (_, _, parcel_name, o) in &objects {
                let owner = if o.group_owned {
                    let groups = world.groups.groups.clone();
                    world.land.group_name(&o.owner, &groups).unwrap_or_default()
                } else {
                    world.social.want_name(o.owner);
                    world.social.name_of(&o.owner)
                };
                let cells = [
                    (aurora_i18n::integer(o.memory / KB), true),
                    (aurora_i18n::integer(o.urls), true),
                    (o.name.clone(), false),
                    (owner, false),
                    (parcel_name.clone(), false),
                    (location_text(o), false),
                ];
                let r = row(ui, p, &widths, &cells, st.selected.contains(&o.id));
                if r.clicked() {
                    // a multi-select list (Ctrl adds)
                    if !ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                        st.selected.clear();
                    }
                    if !st.selected.insert(o.id) {
                        st.selected.remove(&o.id);
                    }
                }
            }
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let size = Vec2::new(0.0, 26.0);
        let loading = reason.is_none() && info.summary.is_none();
        // the button is off until the answer comes (no hammering)
        if widgets::action_button_if(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("ui-land-people-refresh"),
            widgets::ActionKind::Flat,
            size,
            reason.is_none() && !loading,
        )
        .clicked()
        {
            st.pending = true;
            st.selected.clear();
            world.land.scripts.summary = None;
            world.land.scripts.details = None;
        }
        if has_locations
            && widgets::action_button_if(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("ui-land-scripts-highlight"),
                widgets::ActionKind::Flat,
                size,
                !st.selected.is_empty(),
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-scripts-highlight-tip"))
            .clicked()
        {
            // showBeacon: the first selected object's place, tracked
            if let Some((_, _, _, o)) = objects.iter().find(|o| st.selected.contains(&o.3.id))
                && let Some(l) = o.location
            {
                let (ox, oy) = aurora_net::handle_to_origin(v.handle);
                world.map.track_location(ox as f64 + l.x as f64, oy as f64 + l.y as f64, l.z, false);
            }
        }
        if has_local_ids
            && widgets::action_button_if(
                ui,
                p,
                "arrow-back-up",
                aurora_i18n::tr!("ui-land-people-return"),
                widgets::ActionKind::Flat,
                size,
                !st.selected.is_empty(),
            )
            .clicked()
        {
            // returnObjects: one message per parcel, then a refresh
            let mut by_parcel: std::collections::BTreeMap<i32, Vec<Uuid>> = Default::default();
            for (local_id, has_id, _, o) in &objects {
                if has_id.is_some() && st.selected.contains(&o.id) {
                    by_parcel.entry(*local_id).or_default().push(o.id);
                }
            }
            for (local_id, tasks) in by_parcel {
                world.land.return_tasks(local_id, tasks);
            }
            st.pending = true;
            st.selected.clear();
            world.land.scripts.summary = None;
            world.land.scripts.details = None;
        }
    });
}

fn mine_tab(ui: &mut egui::Ui, p: &Palette, st: &mut ScriptsUi, world: &mut World) {
    let mine = world.land.scripts.mine.clone();
    match &mine {
        None => status(ui, p, aurora_i18n::tr!("ui-land-scripts-waiting")),
        Some(Err(e)) => status(ui, p, &format!("{} ({e})", aurora_i18n::tr!("ui-land-scripts-error"))),
        Some(Ok((s, _))) => {
            for line in summary_lines(s) {
                ui.label(RichText::new(line).size(12.5).color(p.ink));
            }
        }
    }
    ui.add_space(4.0);
    let widths = head(
        ui,
        p,
        &[
            (aurora_i18n::tr!("ui-land-scripts-size"), 80.0, true),
            (aurora_i18n::tr!("ui-land-scripts-urls-head"), 50.0, true),
            (aurora_i18n::tr!("common-name"), 0.0, false),
            (aurora_i18n::tr!("ui-land-scripts-attachment"), 0.0, false),
        ],
    );
    let list_h = (ui.available_height() - 40.0).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("land_scripts_mine")
        .max_height(list_h)
        .min_scrolled_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if let Some(Ok((_, attachments))) = &mine {
                for a in attachments {
                    for o in &a.objects {
                        let cells = [
                            (aurora_i18n::integer(o.memory / KB), true),
                            (aurora_i18n::integer(o.urls), true),
                            (o.name.clone(), false),
                            (a.location.clone(), false),
                        ];
                        row(ui, p, &widths, &cells, false);
                    }
                }
            }
        });
    ui.add_space(6.0);
    if widgets::action_button_if(
        ui,
        p,
        "refresh",
        aurora_i18n::tr!("ui-land-people-refresh"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
        mine.is_some(),
    )
    .clicked()
    {
        st.mine_asked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_in_kilobytes() {
        let s = ScriptSummary {
            memory: Some((2 * 1024 * 1024, 8 * 1024 * 1024)),
            urls: Some((3, 750)),
        };
        let lines = summary_lines(&s);
        assert_eq!(
            lines[0],
            aurora_i18n::tr!(
                "ui-land-scripts-memory",
                used = aurora_i18n::integer(2048),
                max = aurora_i18n::integer(8192),
                available = aurora_i18n::integer(6144)
            )
        );
        assert_eq!(lines.len(), 2);
        // no maximum: the simple line
        let simple = summary_lines(&ScriptSummary {
            memory: Some((1024, 0)),
            urls: None,
        });
        assert_eq!(
            simple,
            vec![aurora_i18n::tr!("ui-land-scripts-memory-simple", used = aurora_i18n::integer(1))]
        );
        let o = ScriptObject {
            id: Uuid::nil(),
            name: String::new(),
            owner: Uuid::nil(),
            group_owned: false,
            memory: 0,
            urls: 0,
            location: Some(glam::Vec3::new(140.2, 129.6, 25.0)),
        };
        assert_eq!(location_text(&o), "<140, 130, 25>");
    }
}
