//! Disk: what the viewer reads and writes over the minute, its cache by
//! folder against the size limit, how often the cache already holds what is
//! asked, and the bytes of each origin.

use super::draw::{self, Line, Segment};
use super::memory::two;
use super::snapshot::{HISTORY, Snapshot};
use super::{Action, Monitor, fmt};
use crate::disk_io::Origin;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::vec2;

fn folder_name(dir: &str) -> &'static str {
    match dir {
        "tex" => tr!("common-textures"),
        "mesh" => tr!("monitor-row-meshes"),
        "mat" => tr!("monitor-http-materials"),
        "anim" => tr!("monitor-http-animations"),
        "objects" => tr!("monitor-folder-objects"),
        "inventory" => tr!("monitor-http-inventory"),
        "sound" => tr!("monitor-http-sounds"),
        "settings" => tr!("monitor-http-environments"),
        "clipboard-images" => tr!("ui-chat-preview-clipboard-cache"),
        _ => tr!("monitor-row-other"),
    }
}

fn origin_name(o: Origin) -> &'static str {
    match o {
        Origin::Textures => tr!("monitor-row-tex-cache"),
        Origin::Meshes => tr!("monitor-row-mesh-cache"),
        Origin::Objects => tr!("monitor-row-object-cache"),
        Origin::Inventory => tr!("monitor-row-inventory-cache"),
        Origin::Logs => tr!("monitor-row-logs"),
        Origin::Other => tr!("monitor-origin-other"),
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot, out: &mut Vec<Action>) {
    super::heading(ui, p, tr!("monitor-tab-disk"), tr!("monitor-disk-sub"), |ui| {
        let h = ui.spacing().interact_size.y;
        let clear = tr!("monitor-clear-cache");
        if widgets::action_button(
            ui,
            p,
            "trash-filled",
            clear,
            widgets::ActionKind::Flat,
            vec2(widgets::action_button_fit(ui, "trash-filled", clear), h),
        )
        .clicked()
        {
            out.push(Action::ClearCache);
        }
        let open = tr!("monitor-open-folder");
        if widgets::action_button(
            ui,
            p,
            "folder-open-filled",
            open,
            widgets::ActionKind::Flat,
            vec2(widgets::action_button_fit(ui, "folder-open-filled", open), h),
        )
        .clicked()
        {
            out.push(Action::OpenCacheFolder);
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("monitor-disk")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            activity(ui, p, m, s);
            cache(ui, p, s);
            two(ui, |ui| hits(ui, p, s), |ui| origins(ui, p, s));
        });
}

fn activity(ui: &mut egui::Ui, p: &Palette, m: &Monitor, s: &Snapshot) {
    let reads = m.history.series(|pt| Some(pt.disk_read));
    let writes = m.history.series(|pt| Some(pt.disk_write));
    let top = draw::nice_max(reads.iter().chain(&writes).copied().fold(1024.0 * 1024.0, f64::max) * 1.15);
    let read: f64 = s.disk.rates.iter().map(|r| r[0]).sum();
    let written: f64 = s.disk.rates.iter().map(|r| r[1]).sum();
    draw::card(ui, p, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            draw::text(ui, tr!("monitor-activity"), 13.0, p.ink);
            widgets::status_dot(ui, p.amber);
            draw::text(ui, tr!("monitor-read-value", value = fmt::disk_rate(read)), draw::BODY, p.muted);
            widgets::status_dot(ui, p.rose);
            draw::text(
                ui,
                tr!("monitor-written-value", value = fmt::disk_rate(written)),
                draw::BODY,
                p.muted,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                draw::text(ui, tr!("monitor-scale-minute", scale = fmt::disk_rate(top)), draw::BODY, p.muted);
            });
        });
        ui.add_space(4.0);
        let hover = |i: usize| {
            tr!(
                "monitor-read-written",
                read = fmt::disk_rate(reads.get(i).copied().unwrap_or(0.0)),
                written = fmt::disk_rate(writes.get(i).copied().unwrap_or(0.0))
            )
        };
        draw::graph(
            ui,
            p,
            vec2(ui.available_width(), 80.0),
            &[
                Line {
                    values: &reads,
                    color: p.amber,
                    fill: true,
                },
                Line {
                    values: &writes,
                    color: p.rose,
                    fill: false,
                },
            ],
            top,
            HISTORY,
            Some(&hover),
        );
    });
}

fn cache(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let d = &s.disk;
    draw::card(ui, p, |ui| {
        draw::card_title(
            ui,
            p,
            tr!("monitor-cache-title"),
            &tr!("monitor-x-of-y", x = fmt::bytes(d.used as f64), y = fmt::bytes(d.limit as f64)),
        );
        let colors = draw::part_colors(p);
        let parts: Vec<Segment> = d
            .by_dir
            .iter()
            .enumerate()
            .map(|(i, (_, b))| Segment {
                value: *b as f64,
                color: colors[i % colors.len()],
            })
            .collect();
        draw::segments(ui, p, 16.0, &parts, d.limit as f64);
        ui.add_space(4.0);
        let items: Vec<_> = d
            .by_dir
            .iter()
            .enumerate()
            .map(|(i, (dir, b))| (colors[i % colors.len()], folder_name(dir).to_owned(), fmt::bytes(*b as f64)))
            .collect();
        draw::legend(ui, p, 3, &items);
        if let Some((at, files, freed)) = d.last_trim {
            let ago = at.elapsed().map_or(0, |e| e.as_secs() / 60);
            ui.add_space(4.0);
            draw::text(
                ui,
                tr!(
                    "monitor-last-trim",
                    minutes = ago as i64,
                    files = fmt::count(files),
                    freed = fmt::bytes(freed as f64)
                ),
                draw::SMALL,
                p.muted_dim,
            );
        }
    });
}

fn hits(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let names = [
        tr!("monitor-hits-j2k"),
        tr!("monitor-hits-bc7"),
        tr!("monitor-row-meshes"),
        tr!("monitor-folder-objects"),
    ];
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-hits-title"), tr!("monitor-since-open"));
        for (name, hit) in names.iter().zip(&s.disk.hits) {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::hover());
            let cy = rect.center().y;
            let font = egui::FontId::proportional(draw::BODY);
            widgets::paint_elided(ui, name, egui::pos2(rect.left(), cy), 210.0, font.clone(), p.muted);
            let text = hit.map_or("–".into(), |(pct, _)| fmt::pct(f64::from(pct)));
            let v = ui.painter().layout_no_wrap(text, font, p.ink);
            let vx = rect.right() - v.size().x;
            widgets::paint_row_text(ui.painter(), v, vx, cy, p.ink);
            let meter = egui::Rect::from_min_max(egui::pos2(rect.left() + 218.0, cy - 3.0), egui::pos2(vx - 12.0, cy + 3.0));
            draw::paint_meter(ui, p, meter, hit.map_or(0.0, |h| h.0 / 100.0), p.teal);
            if let Some((_, reads)) = hit {
                ui.interact(rect, ui.scope_id().with(("monitor-hits", name)), egui::Sense::hover())
                    .on_hover_text(tr!("monitor-reads", count = fmt::count(*reads as usize)));
            }
        }
    });
}

fn origins(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-origins-title"), tr!("monitor-last-seconds"));
        ui.spacing_mut().item_spacing.y = 0.0;
        for (o, [r, w]) in Origin::ALL.iter().zip(s.disk.rates) {
            draw::kv_row(
                ui,
                p,
                origin_name(*o),
                &tr!("monitor-read-written", read = fmt::disk_rate(r), written = fmt::disk_rate(w)),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_cache_folder_has_a_name() {
        for d in crate::cache::DIRS {
            assert_ne!(super::folder_name(d), aurora_i18n::tr!("monitor-row-other"), "{d}");
        }
    }
}
