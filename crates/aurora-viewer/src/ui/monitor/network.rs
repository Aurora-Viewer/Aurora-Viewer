//! Network: the traffic of the minute and the link's figures, the HTTP
//! requests by kind, the UDP bandwidth asked of the simulator by channel,
//! the simulators the viewer has a circuit with, and the busiest UDP
//! messages.

use super::draw::{self, Col, Line};
use super::memory::two;
use super::snapshot::{HISTORY, Snapshot};
use super::{Monitor, fmt};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use aurora_net::probe::{HTTP_KINDS, HttpKind};
use egui::{Sense, vec2};

fn kind_name(k: HttpKind) -> &'static str {
    match k {
        HttpKind::Texture => tr!("common-textures"),
        HttpKind::Mesh => tr!("monitor-http-meshes"),
        HttpKind::Animation => tr!("monitor-http-animations"),
        HttpKind::Sound => tr!("monitor-http-sounds"),
        HttpKind::Material => tr!("monitor-http-materials"),
        HttpKind::Settings => tr!("monitor-http-environments"),
        HttpKind::Landmark => tr!("monitor-http-landmarks"),
        HttpKind::Asset => tr!("monitor-http-assets"),
        HttpKind::Inventory => tr!("monitor-http-inventory"),
        HttpKind::Cap => tr!("monitor-http-caps"),
    }
}

/// The UDP channels in the order of `throttle::group`.
fn channels() -> [&'static str; 7] {
    [
        tr!("monitor-channel-resend"),
        tr!("monitor-channel-land"),
        tr!("monitor-channel-wind"),
        tr!("monitor-channel-cloud"),
        tr!("monitor-channel-task"),
        tr!("common-textures"),
        tr!("monitor-channel-asset"),
    ]
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot) {
    let n = &s.net;
    super::heading(
        ui,
        p,
        tr!("monitor-tab-network"),
        &tr!(
            "monitor-network-sub",
            circuits = s.regions.len() as i64,
            active = i64::from(n.http_active)
        ),
        |_| {},
    );
    egui::ScrollArea::vertical()
        .id_salt("monitor-network")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            traffic(ui, p, m, s);
            two(ui, |ui| http(ui, p, s), |ui| throttle(ui, p, s));
            two(ui, |ui| regions(ui, p, s), |ui| messages(ui, p, s));
        });
}

fn traffic(ui: &mut egui::Ui, p: &Palette, m: &Monitor, s: &Snapshot) {
    let n = &s.net;
    let ins = m.history.series(|pt| Some(f64::from(pt.net_in)));
    let outs = m.history.series(|pt| Some(f64::from(pt.net_out)));
    let top = draw::nice_max(ins.iter().chain(&outs).copied().fold(10.0, f64::max) * 1.15);
    draw::card(ui, p, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            draw::text(ui, tr!("ui-perf-throughput"), 13.0, p.ink);
            widgets::status_dot(ui, p.teal);
            draw::text(
                ui,
                tr!("monitor-incoming-value", value = fmt::kbps(f64::from(n.in_kbps))),
                draw::BODY,
                p.muted,
            );
            widgets::status_dot(ui, p.rose);
            draw::text(
                ui,
                tr!("monitor-outgoing-value", value = fmt::kbps(f64::from(n.out_kbps))),
                draw::BODY,
                p.muted,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                draw::text(ui, tr!("monitor-scale-minute", scale = fmt::kbps(top)), draw::BODY, p.muted);
            });
        });
        ui.add_space(4.0);
        let hover = |i: usize| {
            tr!(
                "ui-perf-incoming-outgoing",
                value0 = fmt::kbps(ins.get(i).copied().unwrap_or(0.0)),
                value1 = fmt::kbps(outs.get(i).copied().unwrap_or(0.0))
            )
        };
        draw::graph(
            ui,
            p,
            vec2(ui.available_width(), 90.0),
            &[
                Line {
                    values: &ins,
                    color: p.teal,
                    fill: true,
                },
                Line {
                    values: &outs,
                    color: p.rose,
                    fill: false,
                },
            ],
            top,
            HISTORY,
            Some(&hover),
        );
        ui.add_space(6.0);
        let items = [
            (tr!("ui-perf-ping").to_owned(), format!("{} ms", n.ping_ms)),
            (tr!("ui-perf-losses").to_owned(), fmt::pct(f64::from(n.loss))),
            (
                tr!("monitor-packets-in").to_owned(),
                tr!("monitor-per-second", value = aurora_i18n::number(f64::from(n.packets_in), 0)),
            ),
            (
                tr!("monitor-packets-out").to_owned(),
                tr!("monitor-per-second", value = aurora_i18n::number(f64::from(n.packets_out), 0)),
            ),
            (tr!("monitor-resent-given-up").to_owned(), format!("{} / {}", n.resent, n.given_up)),
            (tr!("ui-perf-unacknowledged").to_owned(), n.unacked.to_string()),
            (
                tr!("ui-perf-input-delay").to_owned(),
                n.input_delay_ms
                    .map_or("–".into(), |v| format!("{} ms", aurora_i18n::number(f64::from(v), 0))),
            ),
            (tr!("ui-perf-network-loop-pause").to_owned(), fmt::ms(f64::from(n.loop_ms))),
            (tr!("ui-perf-event-wait").to_owned(), fmt::ms(f64::from(n.event_wait_ms))),
            (
                tr!("ui-perf-avatar-corrections").to_owned(),
                tr!(
                    "ui-perf-corrections-largest",
                    value0 = n.corrections.0.to_string(),
                    value1 = aurora_i18n::number(f64::from(n.corrections.1), 1)
                ),
            ),
            (
                tr!("ui-perf-downloaded").to_owned(),
                tr!(
                    "monitor-downloaded",
                    size = fmt::bytes(n.downloaded as f64),
                    failed = n.http_failed.to_string()
                ),
            ),
        ];
        draw::figures(ui, p, 8, &items);
    });
}

fn http(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let n = &s.net;
    draw::card(ui, p, |ui| {
        draw::card_title(
            ui,
            p,
            tr!("monitor-http-title"),
            &tr!(
                "ui-perf-active-queued",
                value0 = n.http_active.to_string(),
                value1 = n.http_queued.to_string()
            ),
        );
        let cols = [
            Col::new(tr!("monitor-col-type"), 0.0, false),
            Col::new(tr!("monitor-col-active"), 56.0, true),
            Col::new(tr!("monitor-col-done"), 62.0, true),
            Col::new(tr!("monitor-col-rate"), 80.0, true),
            Col::new(tr!("monitor-col-latency"), 70.0, true),
            Col::new(tr!("monitor-col-failed"), 56.0, true),
        ];
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::header(ui, p, &cols);
        let Some(probes) = &s.probes else {
            ui.add_space(6.0);
            draw::text(ui, tr!("monitor-measuring"), draw::SMALL, p.muted_dim);
            return;
        };
        let secs = f64::from(probes.secs.max(1e-3));
        for (kind, k) in HTTP_KINDS.iter().zip(&probes.http.kinds) {
            if k.started == 0 && k.in_flight == 0 && k.done == 0 && k.bytes == 0 {
                continue;
            }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            draw::row_surface(ui, p, rect, false, false);
            let r = draw::col_ranges(&cols, rect.left(), rect.width());
            let cy = rect.center().y;
            draw::cell_text(ui, r[0], cy, kind_name(*kind), false, p.ink, draw::BODY);
            draw::cell_text(ui, r[1], cy, &k.in_flight.to_string(), true, p.ink, draw::BODY);
            draw::cell_text(
                ui,
                r[2],
                cy,
                &tr!("monitor-per-second", value = aurora_i18n::number(k.done as f64 / secs, 0)),
                true,
                p.ink,
                draw::BODY,
            );
            draw::cell_text(
                ui,
                r[3],
                cy,
                &fmt::kbps(k.bytes as f64 * 8.0 / 1000.0 / secs),
                true,
                p.ink,
                draw::BODY,
            );
            let latency = if k.latency.count() > 0 {
                fmt::ms(f64::from(k.latency.percentile(50.0)))
            } else {
                "–".into()
            };
            draw::cell_text(ui, r[4], cy, &latency, true, p.muted, draw::BODY);
            draw::cell_text(ui, r[5], cy, &k.failed.iter().sum::<u64>().to_string(), true, p.muted, draw::BODY);
        }
    });
}

fn throttle(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let n = &s.net;
    draw::card(ui, p, |ui| {
        let right = if n.throttle_asked > 0 {
            tr!(
                "monitor-throttle-asked",
                asked = fmt::kbps(f64::from(n.throttle_asked)),
                setting = fmt::kbps(f64::from(n.throttle_setting))
            )
        } else {
            tr!("monitor-throttle-setting", setting = fmt::kbps(f64::from(n.throttle_setting)))
        };
        draw::card_title(ui, p, tr!("monitor-throttle-title"), &right);
        let asked = if n.throttle_asked > 0 {
            n.throttle_asked
        } else {
            n.throttle_setting
        };
        let group = aurora_net::throttle::group(asked as f32);
        let largest = group.iter().copied().fold(1.0f32, f32::max);
        for (name, kbps) in channels().iter().zip(group) {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let cy = rect.center().y;
            let font = egui::FontId::proportional(draw::BODY);
            widgets::paint_elided(ui, name, egui::pos2(rect.left(), cy), 90.0, font.clone(), p.muted);
            let v = ui.painter().layout_no_wrap(fmt::kbps(f64::from(kbps)), font, p.ink);
            let vx = rect.right() - v.size().x;
            widgets::paint_row_text(ui.painter(), v, vx, cy, p.ink);
            let meter = egui::Rect::from_min_max(egui::pos2(rect.left() + 98.0, cy - 3.0), egui::pos2(vx - 12.0, cy + 3.0));
            draw::paint_meter(ui, p, meter, kbps / largest, p.violet);
        }
        ui.add_space(4.0);
        draw::text(ui, tr!("monitor-throttle-note"), draw::SMALL, p.muted_dim);
    });
}

fn regions(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    draw::card(ui, p, |ui| {
        draw::card_title(
            ui,
            p,
            tr!("monitor-regions-title"),
            &tr!("monitor-circuits", count = s.regions.len() as i64),
        );
        let cols = [
            Col::new(tr!("monitor-col-region"), 0.0, false),
            Col::new(tr!("monitor-col-role"), 110.0, false),
            Col::new(tr!("monitor-col-dilation"), 80.0, true),
            Col::new(tr!("monitor-col-last-packet"), 100.0, true),
        ];
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::header(ui, p, &cols);
        for r in &s.regions {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            draw::row_surface(ui, p, rect, false, false);
            let c = draw::col_ranges(&cols, rect.left(), rect.width());
            let cy = rect.center().y;
            draw::cell_text(ui, c[0], cy, &r.name, false, p.ink, draw::BODY);
            let role = if r.main {
                tr!("monitor-region-current")
            } else {
                tr!("monitor-region-neighbour")
            };
            draw::cell_text(ui, c[1], cy, role, false, p.muted, draw::BODY);
            let slow = r.dilation < 0.9;
            draw::cell_text(
                ui,
                c[2],
                cy,
                &aurora_i18n::number(f64::from(r.dilation), 2),
                true,
                if slow { p.warn } else { p.ink },
                draw::BODY,
            );
            let age = if r.packet_age.is_finite() {
                if r.packet_age < 1.0 {
                    format!("{} ms", aurora_i18n::number(f64::from(r.packet_age) * 1000.0, 0))
                } else {
                    format!("{} s", aurora_i18n::number(f64::from(r.packet_age), 1))
                }
            } else {
                "–".into()
            };
            draw::cell_text(ui, c[3], cy, &age, true, p.muted, draw::BODY);
        }
    });
}

fn messages(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-messages-title"), tr!("monitor-per-second-unit"));
        let cols = [
            Col::new(tr!("monitor-col-message"), 0.0, false),
            Col::new(tr!("monitor-col-packets"), 70.0, true),
            Col::new(tr!("monitor-col-rate"), 80.0, true),
        ];
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::header(ui, p, &cols);
        let Some(probes) = &s.probes else { return };
        let secs = f64::from(probes.secs.max(1e-3));
        let mut list: Vec<(String, u64, u64)> = probes.messages[0]
            .iter()
            .map(|m| (m.name.to_owned(), m.count, m.bytes))
            .chain(
                probes.messages[1]
                    .iter()
                    .map(|m| (tr!("monitor-outgoing-message", name = m.name), m.count, m.bytes)),
            )
            .collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.2));
        for (name, count, bytes) in list.iter().take(8) {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            draw::row_surface(ui, p, rect, false, false);
            let c = draw::col_ranges(&cols, rect.left(), rect.width());
            let cy = rect.center().y;
            draw::cell_text(ui, c[0], cy, name, false, p.ink, draw::BODY);
            draw::cell_text(ui, c[1], cy, &aurora_i18n::number(*count as f64 / secs, 0), true, p.ink, draw::BODY);
            draw::cell_text(
                ui,
                c[2],
                cy,
                &fmt::kbps(*bytes as f64 * 8.0 / 1000.0 / secs),
                true,
                p.muted,
                draw::BODY,
            );
        }
    });
}
