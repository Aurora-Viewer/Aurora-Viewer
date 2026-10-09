//! "Accès" (LLPanelLandAccess::refresh / refresh_ui / onCommitPublicAccess /
//! onCommitGroupCheck / onCommitAny, access and ban lists, FS:PP export and
//! import as CSV).

use super::general::set;
use super::{Dialog, LandUi, PickFor, View, check, dim};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{add_entry, powers};
use aurora_net::land::{AL_ACCESS, AL_BAN, AccessEntry, PARCEL_MAX_ACCESS_LIST};
use aurora_net::parcel_flags as pf;
use egui::{RichText, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// Ban list duration column (parent floater strings Hours / Minutes...).
pub(super) fn ban_duration(time: i32, now: i64) -> String {
    if time == 0 {
        return "Toujours".into();
    }
    let s = (time as i64 - now).max(0);
    if s >= 7200 {
        format!("{} hrs.", s / 3600)
    } else if s >= 3600 {
        "1 hr.".into()
    } else if s >= 120 {
        format!("{} min", s / 60)
    } else if s >= 60 {
        "1 min".into()
    } else {
        format!("{s} s")
    }
}

/// Prefix of a temporary access entry: " (12 min restantes) ".
pub(super) fn access_prefix(time: i32, now: i64) -> String {
    if time == 0 {
        return String::new();
    }
    let s = (time as i64 - now).max(0);
    let left = if s >= 120 {
        format!("{} min", s / 60)
    } else if s >= 60 {
        "1 min".into()
    } else {
        format!("{s} s")
    };
    format!(" ({left} restantes) ")
}

/// callbackAvatarCBAccess: add one resident (and take them off the ban list).
pub(super) fn add_allowed(world: &mut World, ids: &[Uuid]) {
    let Some(id) = ids.first().copied() else {
        return;
    };
    let Some(sel) = world.land.sel.as_mut() else {
        return;
    };
    let Some(owner) = sel.parcel.as_ref().map(|p| p.owner_id) else {
        return;
    };
    if add_entry(&mut sel.access, &owner, id, 0) {
        let mut which = AL_ACCESS;
        let before = sel.bans.len();
        sel.bans.retain(|e| e.id != id);
        if sel.bans.len() != before {
            which |= AL_BAN;
        }
        sel.revision += 1;
        world.land.send_lists(which);
    }
}

/// callbackAvatarCBBanned2: ban the residents until `time` (0 = always),
/// taking them off the access list.
pub(super) fn add_banned(world: &mut World, ids: &[Uuid], time: i32) {
    let Some(sel) = world.land.sel.as_mut() else {
        return;
    };
    let Some(owner) = sel.parcel.as_ref().map(|p| p.owner_id) else {
        return;
    };
    let mut which = 0;
    for id in ids {
        if add_entry(&mut sel.bans, &owner, *id, time) {
            which |= AL_BAN;
            let before = sel.access.len();
            sel.access.retain(|e| e.id != *id);
            if sel.access.len() != before {
                which |= AL_ACCESS;
            }
        }
    }
    if which != 0 {
        sel.revision += 1;
        world.land.send_lists(which);
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return;
    };
    let f = |b: u32| parcel.flags & b != 0;
    let override_ok = parcel.region_allow_access_override;
    let can_banned = v.can(powers::LAND_MANAGE_BANNED);
    // the estate owner may not let parcel owners manage access
    let can_allowed = override_ok && v.can(powers::LAND_MANAGE_ALLOWED);
    let groups = world.groups.groups.clone();
    let group_name = world.land.group_name(&parcel.group_id, &groups);

    // refresh: what the controls show
    let public = !override_ok || !f(pf::USE_ACCESS_LIST);
    let use_group = override_ok && f(pf::USE_ACCESS_GROUP);
    let payment = parcel.region_deny_anonymous || f(pf::DENY_ANONYMOUS);
    let age = parcel.region_deny_age_unverified || f(pf::DENY_AGEUNVERIFIED);
    let use_pass = f(pf::USE_PASS_LIST);
    if public || !use_pass {
        s.pass_to_group = false;
    }
    let mut now = AccessState {
        public,
        group: use_group,
        payment,
        age,
        pass: use_pass,
        pass_group: s.pass_to_group,
        price: parcel.pass_price,
        hours: parcel.pass_hours,
    };
    let before = now;

    let estate = " (défini par le domaine)";
    let (r, on) = check(ui, can_allowed, now.public, "Tout le monde peut rendre visite");
    r.on_hover_text("Des lignes d'interdiction seront créées si cette case n'est pas cochée");
    if on != now.public {
        now.public = on;
        // FIRE-12551: allow the group when public access goes, or we may ban ourselves
        if !on && group_name.is_some() {
            now.group = true;
        }
    }
    ui.indent("land_access_public", |ui| {
        let (r, on) = check(
            ui,
            now.public && can_allowed && !parcel.region_deny_age_unverified,
            now.age,
            &format!(
                "Doit avoir plus de 18 ans{}",
                if parcel.region_deny_age_unverified { estate } else { "" }
            ),
        );
        r.on_hover_text("Pour accéder à cette parcelle, les résidents doivent avoir au moins 18 ans.");
        now.age = on;
        let (r, on) = check(
            ui,
            now.public && can_allowed && !parcel.region_deny_anonymous,
            now.payment,
            &format!(
                "Les infos de paiement doivent être enregistrées dans le dossier{}",
                if parcel.region_deny_anonymous { estate } else { "" }
            ),
        );
        r.on_hover_text("Pour pouvoir accéder à cette parcelle, les résidents doivent avoir enregistré des informations de paiement.");
        now.payment = on;
    });
    let can_allow_group = !now.public || (now.payment ^ now.age);
    let (r, on) = check(
        ui,
        group_name.is_some() && can_allowed && can_allow_group,
        now.group,
        &format!("Autoriser le groupe {} sans restrictions", group_name.clone().unwrap_or_default()),
    );
    r.on_hover_text("Définir le groupe à l'onglet Général.");
    if on != now.group {
        now.group = on;
        // onCommitGroupCheck: passes "to the group" make no sense with group access
        if on && !now.public {
            now.pass_group = false;
        }
    }
    ui.horizontal(|ui| {
        let (r, on) = check(ui, !now.public && can_allowed, now.pass, "Vendre des pass à :");
        r.on_hover_text("Autoriser un accès temporaire à cette parcelle");
        now.pass = on;
        let pass_on = !now.public && now.pass && can_allowed;
        ui.add_enabled_ui(pass_on, |ui| {
            egui::ComboBox::from_id_salt("land_pass_to")
                .selected_text(if now.pass_group { "Groupe" } else { "Tout le monde" })
                .width(120.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(!now.pass_group, "Tout le monde").clicked() {
                        now.pass_group = false;
                    }
                    if ui.selectable_label(now.pass_group, "Groupe").clicked() {
                        now.pass_group = true;
                    }
                });
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        let pass_on = !now.public && now.pass && can_allowed;
        // spinners send their value once released or typed (Firestorm wires
        // them to the group check by mistake: they only went out with
        // another change)
        let (mut price, mut hours) = s.pass_edit.unwrap_or((now.price, now.hours));
        dim(ui, p, "Prix en L$ :");
        let r = ui.add_enabled(pass_on, egui::DragValue::new(&mut price).range(0..=500));
        ui.add_space(16.0);
        dim(ui, p, "Durée en heures :");
        let r2 = ui.add_enabled(
            pass_on,
            egui::DragValue::new(&mut hours).range(0.0..=24.0).speed(0.1).fixed_decimals(2),
        );
        let active = r.dragged() || r.has_focus() || r2.dragged() || r2.has_focus();
        if active {
            s.pass_edit = Some((price, hours));
        } else if s.pass_edit.take().is_some() || r.changed() || r2.changed() {
            now.price = price;
            now.hours = hours;
        }
    });
    dim(ui, p, "(Le propriétaire de domaine peut avoir limité ces choix)");
    s.pass_to_group = now.pass_group;
    if now != before {
        commit(world, &parcel, &now, group_name.is_some());
    }

    // the two lists
    let sel = world.land.sel.clone();
    let (access, bans) = sel
        .as_ref()
        .map_or((Vec::new(), Vec::new()), |s| (s.access.clone(), s.bans.clone()));
    ui.add_space(4.0);
    let half = (ui.available_width() - 12.0) / 2.0;
    let mut add_access = false;
    let mut add_ban = false;
    let mut remove: Option<(u32, HashSet<Uuid>)> = None;
    let mut export: Option<(&str, Vec<AccessEntry>)> = None;
    let mut import: Option<u32> = None;
    ui.horizontal_top(|ui| {
        for (kind, list) in [(AL_ACCESS, &access), (AL_BAN, &bans)] {
            let can = if kind == AL_ACCESS { can_allowed } else { can_banned };
            let sel_set = if kind == AL_ACCESS { &mut s.access_sel } else { &mut s.ban_sel };
            sel_set.retain(|id| list.iter().any(|e| e.id == *id));
            ui.allocate_ui_with_layout(
                Vec2::new(half, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let title = if kind == AL_ACCESS {
                        format!("Toujours autorisé ({}, max. {PARCEL_MAX_ACCESS_LIST})", list.len())
                    } else {
                        format!("Interdits ({}, max. {PARCEL_MAX_ACCESS_LIST})", list.len())
                    };
                    ui.label(RichText::new(title).size(12.0).color(p.ink));
                    let list_h = (ui.available_height() - 60.0).max(80.0);
                    let mut rows: Vec<(AccessEntry, String)> = list
                        .iter()
                        .map(|e| {
                            world.social.want_name(e.id);
                            (*e, world.social.name_of(&e.id))
                        })
                        .collect();
                    rows.sort_by_key(|r| r.1.to_lowercase());
                    egui::Frame::new().fill(p.field).show(ui, |ui| {
                        ui.set_width(half);
                        ui.set_min_height(list_h);
                        egui::ScrollArea::vertical()
                            .id_salt(("land_list", kind))
                            .max_height(list_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add_enabled_ui(can, |ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    for (e, name) in &rows {
                                        let selected = sel_set.contains(&e.id);
                                        let label = if kind == AL_ACCESS {
                                            format!("{}{name}", access_prefix(e.time, v.now))
                                        } else {
                                            name.clone()
                                        };
                                        let (rect, r) = ui.allocate_exact_size(Vec2::new(half - 14.0, 20.0), egui::Sense::click());
                                        let fill = if selected {
                                            p.violet.gamma_multiply(0.6)
                                        } else if r.hovered() {
                                            p.raised
                                        } else {
                                            p.field
                                        };
                                        ui.painter().rect_filled(rect, 0.0, fill);
                                        let dur_w = if kind == AL_BAN { 70.0 } else { 0.0 };
                                        let mut job =
                                            egui::text::LayoutJob::simple_singleline(label, egui::FontId::proportional(12.0), p.ink);
                                        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - dur_w - 10.0);
                                        let g = ui.painter().layout_job(job);
                                        ui.painter()
                                            .galley(egui::pos2(rect.left() + 5.0, rect.center().y - g.size().y / 2.0), g, p.ink);
                                        if kind == AL_BAN {
                                            ui.painter().text(
                                                egui::pos2(rect.right() - dur_w, rect.center().y),
                                                egui::Align2::LEFT_CENTER,
                                                ban_duration(e.time, v.now),
                                                egui::FontId::proportional(11.5),
                                                p.muted,
                                            );
                                        }
                                        if r.clicked() {
                                            // multi-select with Ctrl, like the name lists
                                            if !ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                                                sel_set.clear();
                                            }
                                            if !sel_set.insert(e.id) {
                                                sel_set.remove(&e.id);
                                            }
                                        }
                                        if r.double_clicked() {
                                            super::super::profile::request_open(ui.ctx(), e.id);
                                        }
                                    }
                                });
                            });
                    });
                    ui.horizontal(|ui| {
                        if super::button(ui, p, "Ajouter", can && list.len() < PARCEL_MAX_ACCESS_LIST).clicked() {
                            if kind == AL_ACCESS {
                                add_access = true;
                            } else {
                                add_ban = true;
                            }
                        }
                        if super::button(ui, p, "Supprimer", can && !sel_set.is_empty()).clicked() {
                            remove = Some((kind, sel_set.clone()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::button(ui, p, "Importer", can && list.len() < PARCEL_MAX_ACCESS_LIST)
                                .on_hover_text("Importer un fichier CSV contenant uniquement des UUID valides, un par ligne.")
                                .clicked()
                            {
                                import = Some(kind);
                            }
                            if super::button(ui, p, "Exporter", can && !list.is_empty()).clicked() {
                                let file = if kind == AL_ACCESS {
                                    "land_access_list.csv"
                                } else {
                                    "land_banned_list.csv"
                                };
                                export = Some((file, list.clone()));
                            }
                        });
                    });
                },
            );
            ui.add_space(12.0);
        }
    });
    if add_access {
        s.pick_for = Some(PickFor::Access);
        s.picker.open("land_avatar_picker", false);
    }
    if add_ban {
        s.pick_for = Some(PickFor::Ban);
        s.picker.open("land_avatar_picker", true);
    }
    if let Some((kind, ids)) = remove
        && let Some(sel) = world.land.sel.as_mut()
    {
        if kind == AL_ACCESS {
            sel.access.retain(|e| !ids.contains(&e.id));
            s.access_sel.clear();
        } else {
            sel.bans.retain(|e| !ids.contains(&e.id));
            s.ban_sel.clear();
        }
        sel.revision += 1;
        world.land.send_lists(kind);
    }
    if let Some((file, list)) = export {
        let csv = export_csv(&list, |id| world.social.name_of(id));
        s.file_job = Some(super::files::save(file, csv));
    }
    if let Some(kind) = import {
        s.file_job = Some(super::files::open(kind));
    }
    if let Some(done) = s.file_job.as_ref().and_then(|j| j.try_take()) {
        s.file_job = None;
        match done {
            super::files::Done::Saved(path) => {
                s.dialog = Some(Dialog::Message(format!("Exportation terminée et sauvegardée dans {path}.")));
            }
            super::files::Done::Opened(kind, text) => import_list(s, world, kind, &text),
            super::files::Done::Failed(e) => s.dialog = Some(Dialog::Message(e)),
            super::files::Done::Cancelled => {}
        }
    }
}

/// The access controls as the panel shows them.
#[derive(Clone, Copy, PartialEq)]
struct AccessState {
    public: bool,
    group: bool,
    payment: bool,
    age: bool,
    pass: bool,
    pass_group: bool,
    price: i32,
    hours: f32,
}

/// onCommitAny: the access flags and pass settings.
fn commit(world: &mut World, parcel: &aurora_net::ParcelInfo, a: &AccessState, group_known: bool) {
    let mut use_group = a.group && group_known;
    let (mut payment, mut age, mut access_list, mut pass) = (false, false, false, false);
    if a.public {
        payment = a.payment && !parcel.region_deny_anonymous;
        age = a.age && !parcel.region_deny_age_unverified;
    } else {
        access_list = true;
        pass = a.pass;
        if use_group && pass && a.pass_group {
            use_group = false;
        }
    }
    let (price, hours) = (a.price, a.hours);
    world.land.update(|u| {
        let mut f = u.flags;
        f = set(f, pf::USE_ACCESS_GROUP, use_group);
        f = set(f, pf::USE_ACCESS_LIST, access_list);
        f = set(f, pf::USE_PASS_LIST, pass);
        f = set(f, pf::USE_BAN_LIST, true);
        f = set(f, pf::DENY_ANONYMOUS, payment);
        f = set(f, pf::DENY_AGEUNVERIFIED, age);
        u.flags = f;
        u.pass_price = price;
        u.pass_hours = hours;
    });
}

/// exportListCallback: "Name,UUID" then one line per entry.
pub(super) fn export_csv(list: &[AccessEntry], name: impl Fn(&Uuid) -> String) -> String {
    let mut out = String::from("Name,UUID\n");
    for e in list {
        out.push_str(&format!("{},{}\n", name(&e.id), e.id));
    }
    out
}

/// ll_sd_from_csv + the "UUID" column of each row.
pub(super) fn import_csv(text: &str) -> Vec<Uuid> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<&str> = header.split(',').map(|c| c.trim().trim_matches('"')).collect();
    let Some(at) = cols.iter().position(|c| *c == "UUID") else {
        return Vec::new();
    };
    lines
        .filter_map(|l| l.split(',').nth(at))
        .filter_map(|v| Uuid::parse_str(v.trim().trim_matches('"')).ok())
        .filter(|id| !id.is_nil())
        .collect()
}

/// importListCallback.
fn import_list(s: &mut LandUi, world: &mut World, kind: u32, text: &str) {
    let ids = import_csv(text);
    if ids.is_empty() {
        s.dialog = Some(Dialog::Message("Aucun UUID valide trouvé dans le fichier.".into()));
        return;
    }
    let Some(sel) = world.land.sel.as_mut() else {
        return;
    };
    let Some(owner) = sel.parcel.as_ref().map(|p| p.owner_id) else {
        return;
    };
    let list = if kind == AL_BAN { &mut sel.bans } else { &mut sel.access };
    let available = PARCEL_MAX_ACCESS_LIST.saturating_sub(list.len());
    if ids.len() > available {
        s.dialog = Some(Dialog::Message(format!(
            "La liste importée contient {} entrées, mais seules {available} places sont disponibles.",
            ids.len()
        )));
        return;
    }
    for id in &ids {
        add_entry(list, &owner, *id, 0);
    }
    sel.revision += 1;
    world.land.send_lists(kind);
    s.dialog = Some(Dialog::Message(format!("{} entrées importées.", ids.len())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_like_the_floater() {
        assert_eq!(ban_duration(0, 0), "Toujours");
        assert_eq!(ban_duration(3 * 3600 + 5, 0), "3 hrs.");
        assert_eq!(ban_duration(3700, 0), "1 hr.");
        assert_eq!(ban_duration(600, 0), "10 min");
        assert_eq!(ban_duration(30, 0), "30 s");
        assert_eq!(ban_duration(10, 100), "0 s");
        assert_eq!(access_prefix(300, 0), " (5 min restantes) ");
    }

    #[test]
    fn csv_round_trip() {
        let id = Uuid::from_u128(0x1234);
        let list = [AccessEntry { id, time: 0, flags: 0 }];
        let csv = export_csv(&list, |_| "Tess Touch".into());
        assert_eq!(csv, format!("Name,UUID\nTess Touch,{id}\n"));
        assert_eq!(import_csv(&csv), vec![id]);
        assert!(import_csv("nothing\nhere").is_empty());
    }
}
