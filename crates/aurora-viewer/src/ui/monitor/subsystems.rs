//! Subsystems: a task manager's table of the parts of the viewer, what
//! each costs in the seven columns of the model, a part unfolding into its
//! sub-parts; sorted by any column, filtered by name, the processor as a
//! share of the machine or of one core. Under it, the selected row: what
//! the part does, its threads, the settings that change its cost, and its
//! processor over the minute.

use super::draw::{self, Col, Line, ROW_H};
use super::model::{Column, RowId, Usage};
use super::snapshot::{HISTORY, Snapshot};
use super::{Action, Monitor, Page, Selected, fmt};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{Rect, Sense, pos2, vec2};

pub fn row_name(r: RowId) -> &'static str {
    match r {
        RowId::Render => tr!("monitor-row-render"),
        RowId::RenderShadows => tr!("ui-perf-shadows-and-depth"),
        RowId::RenderObjects => tr!("ui-perf-objects-and-avatars"),
        RowId::RenderTerrain => tr!("ui-perf-terrain-and-sky"),
        RowId::RenderWater => tr!("monitor-row-water"),
        RowId::RenderBlend => tr!("ui-perf-transparent-objects-and-particles"),
        RowId::RenderPost => tr!("monitor-row-post"),
        RowId::RenderSubmit => tr!("monitor-row-submit"),
        RowId::Textures => tr!("common-textures"),
        RowId::TexFetch => tr!("monitor-row-download"),
        RowId::TexDecode => tr!("monitor-row-tex-decode"),
        RowId::TexUpload => tr!("monitor-row-upload"),
        RowId::TexPages => tr!("ui-perf-texture-pages"),
        RowId::TexCache => tr!("monitor-row-tex-cache"),
        RowId::Meshes => tr!("monitor-row-meshes"),
        RowId::MeshFetch => tr!("monitor-row-download"),
        RowId::MeshDecode => tr!("monitor-row-mesh-decode"),
        RowId::MeshGpu => tr!("monitor-row-mesh-gpu"),
        RowId::MeshCache => tr!("monitor-row-mesh-cache"),
        RowId::Avatars => tr!("monitor-row-avatars"),
        RowId::AvPoses => tr!("monitor-row-poses"),
        RowId::AvComplexity => tr!("monitor-row-complexity"),
        RowId::AvAnimations => tr!("monitor-row-animations"),
        RowId::AvImpostors => tr!("monitor-row-impostors"),
        RowId::Scene => tr!("monitor-row-scene"),
        RowId::SceneCull => tr!("ui-perf-culling-and-lists"),
        RowId::SceneSync => tr!("monitor-row-sync"),
        RowId::SceneParticles => tr!("common-particles"),
        RowId::SceneLights => tr!("monitor-row-lights"),
        RowId::SceneCache => tr!("monitor-row-object-cache"),
        RowId::World => tr!("monitor-row-world"),
        RowId::WorldIn => tr!("monitor-row-received"),
        RowId::WorldOut => tr!("monitor-row-sent"),
        RowId::WorldEvents => tr!("monitor-row-events"),
        RowId::Interface => tr!("common-interface"),
        RowId::UiBars => tr!("monitor-row-bars"),
        RowId::UiTags => tr!("monitor-row-tags"),
        RowId::UiChat => tr!("monitor-row-chat"),
        RowId::UiPeople => tr!("monitor-row-people"),
        RowId::UiInventory => tr!("monitor-row-inventory"),
        RowId::UiMenus => tr!("monitor-row-menus"),
        RowId::UiPlaces => tr!("monitor-row-places"),
        RowId::UiMaps => tr!("monitor-row-maps"),
        RowId::UiMonitor => tr!("monitor-title"),
        RowId::UiOptions => tr!("monitor-row-options"),
        RowId::UiPass => tr!("monitor-row-ui-pass"),
        RowId::UiOther => tr!("monitor-row-ui-other"),
        RowId::Media => tr!("monitor-row-media"),
        RowId::Audio => tr!("monitor-row-audio"),
        RowId::AudioMixer => tr!("monitor-row-mixer"),
        RowId::AudioStream => tr!("monitor-row-stream"),
        RowId::Voice => tr!("monitor-row-voice"),
        RowId::Services => tr!("monitor-row-services"),
        RowId::SvcInventory => tr!("monitor-row-inventory-cache"),
        RowId::SvcCaps => tr!("monitor-row-caps"),
        RowId::SvcLogs => tr!("monitor-row-logs"),
        RowId::CacheUpkeep => tr!("monitor-row-cache-upkeep"),
        RowId::Other => tr!("monitor-row-other"),
    }
}

fn row_icon(r: RowId) -> &'static str {
    match r {
        RowId::Render => "brush",
        RowId::Textures => "photo-filled",
        RowId::Meshes => "cube",
        RowId::Avatars => "users",
        RowId::Scene => "stack-2-filled",
        RowId::World => "network",
        RowId::Interface => "layout-dashboard-filled",
        RowId::Media => "movie",
        RowId::Audio => "music",
        RowId::Voice => "microphone-filled",
        RowId::Services => "list-details-filled",
        RowId::CacheUpkeep => "database-filled",
        _ => "hourglass-filled",
    }
}

fn row_desc(r: RowId) -> &'static str {
    match r {
        RowId::Render => tr!("monitor-desc-render"),
        RowId::Textures => tr!("monitor-desc-textures"),
        RowId::Meshes => tr!("monitor-desc-meshes"),
        RowId::Avatars => tr!("monitor-desc-avatars"),
        RowId::Scene => tr!("monitor-desc-scene"),
        RowId::World => tr!("monitor-desc-world"),
        RowId::Interface => tr!("monitor-desc-interface"),
        RowId::Media => tr!("monitor-desc-media"),
        RowId::Audio => tr!("monitor-desc-audio"),
        RowId::Voice => tr!("monitor-desc-voice"),
        RowId::Services => tr!("monitor-desc-services"),
        RowId::CacheUpkeep => tr!("monitor-desc-cache"),
        _ => tr!("monitor-desc-other"),
    }
}

/// The groups of threads that work for a part.
fn row_threads(r: RowId) -> &'static [&'static str] {
    match r {
        RowId::Render => &["aurora-render", "aurora-render-finish", "aurora-worker"],
        RowId::Textures => &["aurora-jobs", "aurora-pages", "aurora-texcache", "aurora-net"],
        RowId::Meshes => &["aurora-jobs", "aurora-net"],
        RowId::Avatars => &["main", "aurora-worker"],
        RowId::Scene => &["main", "aurora-worker", "aurora-trash"],
        RowId::World => &["aurora-net", "main"],
        RowId::Interface => &["main", "aurora-render", "aurora-monitor"],
        RowId::Media => &["SLPlugin"],
        RowId::Audio => &["aurora-audio-out", "aurora-audio-stream"],
        RowId::Voice => &["aurora-voice", "aurora-voice-rt", "aurora-audio-in"],
        RowId::Services => &["aurora-inventory-cache", "aurora-chatlog", "aurora-notification-history"],
        RowId::CacheUpkeep => &["aurora-cache"],
        _ => &["main", "unnamed"],
    }
}

/// The settings that change what a part costs, and their page.
fn row_settings(r: RowId) -> &'static [(&'static str, Page)] {
    match r {
        RowId::Render => &[
            ("monitor-setting-preset", Page::Graphics),
            ("monitor-setting-shadows", Page::Graphics),
            ("monitor-setting-reflections", Page::Graphics),
            ("monitor-setting-antialiasing", Page::Graphics),
        ],
        RowId::Textures => &[
            ("monitor-setting-texture-budget", Page::Graphics),
            ("monitor-setting-draw-distance", Page::Graphics),
        ],
        RowId::Meshes => &[
            ("monitor-setting-lod", Page::Graphics),
            ("monitor-setting-draw-distance", Page::Graphics),
        ],
        RowId::Avatars => &[
            ("monitor-setting-complexity", Page::Graphics),
            ("monitor-setting-full-avatars", Page::Graphics),
        ],
        RowId::Scene => &[
            ("monitor-setting-particles", Page::Graphics),
            ("monitor-setting-probes", Page::Graphics),
        ],
        RowId::World => &[("monitor-setting-bandwidth", Page::Network)],
        RowId::Interface => &[("monitor-setting-ui-scale", Page::Interface)],
        RowId::Media => &[("monitor-setting-media", Page::Audio)],
        RowId::Audio => &[("monitor-setting-sounds", Page::Audio)],
        RowId::Voice => &[("monitor-setting-voice", Page::Audio)],
        RowId::Services => &[("monitor-setting-chat-logs", Page::Chat)],
        RowId::CacheUpkeep => &[("monitor-setting-cache-size", Page::Network)],
        _ => &[],
    }
}

fn setting_label(key: &str) -> &'static str {
    match key {
        "monitor-setting-preset" => tr!("monitor-setting-preset"),
        "monitor-setting-shadows" => tr!("monitor-setting-shadows"),
        "monitor-setting-reflections" => tr!("monitor-setting-reflections"),
        "monitor-setting-antialiasing" => tr!("monitor-setting-antialiasing"),
        "monitor-setting-texture-budget" => tr!("monitor-setting-texture-budget"),
        "monitor-setting-draw-distance" => tr!("monitor-setting-draw-distance"),
        "monitor-setting-lod" => tr!("monitor-setting-lod"),
        "monitor-setting-complexity" => tr!("monitor-setting-complexity"),
        "monitor-setting-full-avatars" => tr!("monitor-setting-full-avatars"),
        "monitor-setting-particles" => tr!("monitor-setting-particles"),
        "monitor-setting-probes" => tr!("monitor-setting-probes"),
        "monitor-setting-bandwidth" => tr!("monitor-setting-bandwidth"),
        "monitor-setting-ui-scale" => tr!("monitor-setting-ui-scale"),
        "monitor-setting-media" => tr!("monitor-setting-media"),
        "monitor-setting-sounds" => tr!("monitor-setting-sounds"),
        "monitor-setting-voice" => tr!("monitor-setting-voice"),
        "monitor-setting-chat-logs" => tr!("monitor-setting-chat-logs"),
        _ => tr!("monitor-setting-cache-size"),
    }
}

/// A column's title.
pub fn column_title(c: Column, per_core: bool) -> &'static str {
    match c {
        Column::Cpu if per_core => tr!("monitor-col-cpu-core"),
        Column::Cpu => tr!("monitor-tile-cpu"),
        Column::Ms => tr!("monitor-col-frame-time"),
        Column::Gpu => tr!("monitor-col-gpu"),
        Column::Ram => tr!("monitor-col-ram"),
        Column::Vram => tr!("monitor-col-vram"),
        Column::Net => tr!("monitor-tile-net"),
        Column::Disk => tr!("monitor-tile-disk"),
    }
}

fn column_hint(c: Column) -> &'static str {
    match c {
        Column::Cpu => tr!("monitor-col-cpu-hint"),
        Column::Ms => tr!("monitor-col-frame-time-hint"),
        Column::Gpu => tr!("monitor-col-gpu-hint"),
        Column::Ram => tr!("monitor-col-ram-hint"),
        Column::Vram => tr!("monitor-col-vram-hint"),
        Column::Net => tr!("monitor-col-net-hint"),
        Column::Disk => tr!("monitor-col-disk-hint"),
    }
}

/// A value of a column, in its unit.
pub fn format(c: Column, v: f64) -> String {
    match c {
        Column::Cpu => fmt::pct(v),
        Column::Ms | Column::Gpu => fmt::ms(v),
        Column::Ram | Column::Vram => fmt::bytes(v),
        Column::Net => fmt::kbps(v),
        Column::Disk => fmt::disk_rate(v),
    }
}

/// One line of the table.
struct Entry {
    name: String,
    usage: Usage,
    depth: u8,
    icon: Option<&'static str>,
    /// Folded or not, for a part with sub-parts.
    fold: Option<bool>,
    row: Option<RowId>,
    select: Selected,
    badge: Option<String>,
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot, out: &mut Vec<Action>) {
    let cores = s.cpu.as_ref().map_or(1, |c| c.cores.max(1)) as f64;
    let any_open = !m.open.is_empty();
    super::heading(ui, p, tr!("monitor-tab-subsystems"), tr!("monitor-subsystems-sub"), |ui| {
        let all = if any_open {
            tr!("monitor-fold-all")
        } else {
            tr!("monitor-unfold-all")
        };
        if widgets::flat_button(ui, p, all).clicked() {
            if any_open {
                m.open.clear();
            } else {
                m.open = RowId::parents()
                    .filter(|r| r.kids().next().is_some() || *r == RowId::Media)
                    .collect();
            }
        }
        let mut unit = usize::from(m.per_core);
        if widgets::segmented(
            ui,
            p,
            &mut unit,
            &[(tr!("monitor-of-machine"), true), (tr!("monitor-of-one-core"), true)],
        ) {
            m.per_core = unit == 1;
        }
        widgets::search_field(ui, &mut m.filter, tr!("monitor-filter"), 150.0);
    });

    let per_core = m.per_core;
    let scale = move |c: Column, v: f64| if c == Column::Cpu && per_core { v * cores } else { v };
    let lines = lines(m, s);
    let max: Vec<f64> = Column::ALL
        .iter()
        .map(|c| {
            RowId::parents()
                .map(|r| s.attribution.get(r).or_zero(*c))
                .chain(s.attribution.media.iter().map(|m| m.1.or_zero(*c)))
                .fold(0.0, f64::max)
        })
        .collect();

    let titles: Vec<&str> = Column::ALL.iter().map(|c| column_title(*c, m.per_core)).collect();
    let mut cols = vec![Col::new(tr!("monitor-col-subsystem"), 0.0, false).total("Aurora".into())];
    for (c, title) in Column::ALL.iter().zip(&titles) {
        let total = s.attribution.total(*c).map_or("–".into(), |v| format(*c, scale(*c, v)));
        cols.push(Col::new(title, 96.0, true).sort(m.sort == *c).total(total));
    }
    // the table in exactly the room the detail leaves
    let (mut table, foot) = widgets::above_footer(ui);
    let frame = egui::Frame::new()
        .fill(p.bar)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(table.visuals().window_corner_radius);
    let table_h = (table.available_height() - 2.0 * frame.stroke.width).max(0.0);
    frame.show(&mut table, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(table_h);
        ui.spacing_mut().item_spacing.y = 0.0;
        if let Some(i) = draw::header(ui, p, &cols)
            && i > 0
        {
            m.sort = Column::ALL[i - 1];
        }
        // (hints on the column titles)
        let head = ui.min_rect();
        for (i, c) in Column::ALL.iter().enumerate() {
            let ranges = draw::col_ranges(&cols, head.left(), head.width());
            let r = Rect::from_min_max(pos2(ranges[i + 1].0, head.top()), pos2(ranges[i + 1].1, head.bottom()));
            ui.interact(r, ui.scope_id().with(("monitor-col-hint", i)), Sense::hover())
                .on_hover_text(column_hint(*c));
        }
        egui::ScrollArea::vertical()
            .id_salt("monitor-subsystems")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &lines {
                    table_row(ui, p, m, &cols, line, &max, &scale);
                }
            });
    });
    foot.show(ui, |ui| detail(ui, p, m, s, cores, out));
}

/// The table's lines: the parts sorted by the column, each followed by its
/// sub-parts when unfolded (or when the filter finds them).
fn lines(m: &Monitor, s: &Snapshot) -> Vec<Entry> {
    let a = &s.attribution;
    let q = m.filter.trim().to_lowercase();
    let matches = |name: &str| q.is_empty() || name.to_lowercase().contains(&q);
    let by = |u: &Usage| u.or_zero(m.sort);
    let mut parents: Vec<RowId> = RowId::parents().collect();
    parents.sort_by(|x, y| by(&a.get(*y)).total_cmp(&by(&a.get(*x))));
    let mut out = Vec::new();
    for parent in parents {
        let mut kids: Vec<Entry> = parent
            .kids()
            .map(|k| Entry {
                name: row_name(k).to_owned(),
                usage: a.get(k),
                depth: 1,
                icon: None,
                fold: None,
                row: Some(k),
                select: Selected::Row(k),
                badge: None,
            })
            .collect();
        if parent == RowId::Media {
            kids = a
                .media
                .iter()
                .map(|(proc_, usage)| Entry {
                    name: proc_.label.clone(),
                    usage: *usage,
                    depth: 1,
                    icon: None,
                    fold: None,
                    row: None,
                    select: Selected::Media(proc_.pid),
                    badge: Some(if !proc_.video {
                        "CEF".into()
                    } else if cfg!(target_os = "linux") {
                        "GStreamer".into()
                    } else {
                        "libvlc".into()
                    }),
                })
                .collect();
        }
        kids.sort_by(|x, y| by(&y.usage).total_cmp(&by(&x.usage)));
        let name = row_name(parent);
        let found: Vec<Entry> = kids.into_iter().filter(|k| matches(&k.name) || matches(name)).collect();
        if !matches(name) && found.is_empty() {
            continue;
        }
        let has_kids = !found.is_empty() || parent.kids().next().is_some();
        let open = m.open.contains(&parent) || (!q.is_empty() && !found.is_empty());
        let badge = (parent == RowId::Media && !a.media.is_empty()).then(|| tr!("monitor-processes", count = a.media.len() as i64));
        out.push(Entry {
            name: name.to_owned(),
            usage: a.get(parent),
            depth: 0,
            icon: Some(row_icon(parent)),
            fold: has_kids.then_some(open),
            row: Some(parent),
            select: Selected::Row(parent),
            badge,
        });
        if open {
            out.extend(found);
        }
    }
    out
}

fn table_row(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, cols: &[Col], line: &Entry, max: &[f64], scale: &dyn Fn(Column, f64) -> f64) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let ranges = draw::col_ranges(cols, rect.left(), rect.width());
    let selected = m.selected == line.select;
    draw::row_surface(ui, p, rect, selected, resp.hovered());
    let cy = rect.center().y;
    // values, tinted by their share of the column's largest
    for (i, c) in Column::ALL.iter().enumerate() {
        let (l, r) = ranges[i + 1];
        match line.usage.get(*c) {
            Some(v) if v > 0.0005 => {
                let cell = Rect::from_min_max(pos2(l, rect.top()), pos2(r, rect.bottom() - 1.0));
                if max[i] > 0.0 {
                    ui.painter().rect_filled(cell, 0.0, draw::heat(p, v / max[i]));
                }
                draw::cell_text(ui, (l, r), cy, &format(*c, scale(*c, v)), true, p.ink, draw::BODY);
            }
            _ => draw::cell_text(ui, (l, r), cy, "–", true, p.muted_dim, draw::BODY),
        }
    }
    // the name, its caret, its icon
    let (l, r) = ranges[0];
    let mut x = l + 6.0 + if line.depth > 0 { 24.0 } else { 0.0 };
    if let Some(open) = line.fold {
        let caret = Rect::from_center_size(pos2(x + 9.0, cy), vec2(18.0, 18.0));
        let c = ui.interact(caret, ui.scope_id().with(("monitor-fold", &line.name)), Sense::click());
        if c.hovered() {
            ui.painter().rect_filled(caret, widgets::inner_radius(ui.visuals()), p.raised);
        }
        widgets::paint_glyph(
            ui.painter(),
            if open { "chevron-down" } else { "chevron-right" },
            caret.center(),
            13.0,
            if c.hovered() { p.ink } else { p.muted },
        );
        if c.on_hover_text(if open { tr!("common-collapse") } else { tr!("common-expand") })
            .clicked()
            && let Some(row) = line.row
            && !m.open.remove(&row)
        {
            m.open.insert(row);
        }
    }
    x += 24.0;
    if let Some(icon) = line.icon {
        widgets::paint_glyph(ui.painter(), icon, pos2(x + 8.0, cy), 16.0, p.violet_light);
        x += 24.0;
    }
    let mut end = r - 8.0;
    if let Some(badge) = &line.badge {
        let w = widgets::row_tag_width(ui, badge);
        end -= w + 6.0;
        widgets::paint_row_tag(ui, p, badge, false, end + 6.0, cy);
    }
    let color = if line.depth > 0 { p.muted } else { p.ink };
    let name_color = if selected { p.ink } else { color };
    widgets::paint_elided(
        ui,
        &line.name,
        pos2(x, cy),
        (end - x).max(20.0),
        egui::FontId::proportional(draw::BODY),
        name_color,
    );
    if resp.clicked() {
        m.selected = line.select;
    }
}

/// The selected row: what it is, its threads, the settings that change its
/// cost, its figures and its processor over the minute.
fn detail(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot, cores: f64, out: &mut Vec<Action>) {
    let a = &s.attribution;
    let (title, parent, usage, series) = match m.selected {
        Selected::Row(r) => (row_name(r).to_owned(), r.parent(), a.get(r), m.history.row_series(r)),
        Selected::Media(pid) => match a.media.iter().find(|x| x.0.pid == pid) {
            Some((proc_, u)) => (proc_.label.clone(), Some(RowId::Media), *u, Vec::new()),
            None => (
                row_name(RowId::Media).to_owned(),
                None,
                a.get(RowId::Media),
                m.history.row_series(RowId::Media),
            ),
        },
    };
    let part = match m.selected {
        Selected::Row(r) => r.parent().unwrap_or(r),
        Selected::Media(_) => RowId::Media,
    };
    let per_core = m.per_core;
    let scale = |c: Column, v: f64| if c == Column::Cpu && per_core { v * cores } else { v };
    draw::card(ui, p, |ui| {
        ui.set_min_height(130.0);
        ui.horizontal_top(|ui| {
            let left = (ui.available_width() - 14.0) * 0.56;
            ui.allocate_ui_with_layout(vec2(left, 130.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(left);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    widgets::glyph(ui, row_icon(part), p.violet_light, 16.0);
                    draw::text(ui, &title, 13.0, p.ink);
                    if let Some(parent) = parent {
                        draw::text(ui, tr!("monitor-in-part", part = row_name(parent)), draw::BODY, p.muted_dim);
                    }
                });
                ui.add(egui::Label::new(egui::RichText::new(row_desc(part)).size(draw::BODY).color(p.muted)).wrap());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    draw::text(ui, tr!("monitor-threads"), draw::SMALL, p.muted_dim);
                    for name in row_threads(part) {
                        let n = s
                            .cpu
                            .as_ref()
                            .and_then(|c| c.groups.iter().find(|g| g.group == *name))
                            .map(|g| g.threads);
                        let chip = match n {
                            Some(n) if n > 1 => format!("{name} ×{n}"),
                            _ => (*name).to_owned(),
                        };
                        let w = widgets::row_tag_width(ui, &chip);
                        let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                        widgets::paint_row_tag(ui, p, &chip, false, rect.left(), rect.center().y);
                    }
                });
                let settings = row_settings(part);
                if !settings.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        draw::text(ui, tr!("monitor-related-settings"), draw::SMALL, p.muted_dim);
                        for (key, page) in settings {
                            if draw::link(ui, p, setting_label(key)).clicked() {
                                out.push(Action::Preferences(*page));
                            }
                        }
                    });
                }
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let total_cpu = a.total(Column::Cpu).unwrap_or(0.0);
                let mut items: Vec<(String, String)> = Column::ALL
                    .iter()
                    .map(|c| {
                        (
                            column_title(*c, per_core).to_owned(),
                            usage.get(*c).map_or("–".into(), |v| format(*c, scale(*c, v))),
                        )
                    })
                    .collect();
                items.push((
                    tr!("monitor-cpu-share").to_owned(),
                    usage
                        .get(Column::Cpu)
                        .filter(|_| total_cpu > 0.0)
                        .map_or("–".into(), |v| fmt::pct(v / total_cpu * 100.0)),
                ));
                draw::figures(ui, p, 4, &items);
                if !series.is_empty() {
                    let scaled: Vec<f64> = series.iter().map(|v| scale(Column::Cpu, *v)).collect();
                    let top = draw::nice_max(scaled.iter().copied().fold(0.1, f64::max) * 1.2);
                    draw::graph(
                        ui,
                        p,
                        vec2(ui.available_width(), 34.0),
                        &[Line {
                            values: &scaled,
                            color: p.violet_light,
                            fill: true,
                        }],
                        top,
                        HISTORY,
                        Some(&|i: usize| scaled.get(i).map_or(String::new(), |v| fmt::pct(*v))),
                    );
                    draw::text(ui, tr!("monitor-cpu-graph", scale = fmt::pct(top)), draw::SMALL, p.muted_dim);
                }
            });
        });
    });
}
