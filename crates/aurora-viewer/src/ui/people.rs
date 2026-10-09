//! "Personnes" floater (nearby / friends / groups / blocked).

use super::Panels;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::mutes::{MuteType, flag};
use egui::{Color32, RichText, Vec2};
use glam::Vec3;
use uuid::Uuid;

pub enum PeopleAction {
    OpenIm(Uuid),
    /// "Voir le profil" (menu_people_nearby.xml / friends, Avatar.Profile).
    Profile(Uuid),
    OfferTeleport(Uuid),
    /// Open the chat of one of our groups.
    GroupChat(Uuid),
    /// Block / unblock a group's chat (exoGroupMuteList).
    GroupChatBlocked(Uuid, bool),
    /// Block (true) or unblock a resident.
    BlockAvatar(Uuid, String, bool),
    /// Remove a block list entry (by id, or by name for legacy entries).
    Unblock(Uuid, String),
    /// Block an object by its name (LLFloaterGetBlockedObjectName).
    BlockByName(String),
    /// Mute (true) / unmute one property of an entry (mutes::flag).
    BlockFlag {
        id: Uuid,
        name: String,
        kind: MuteType,
        flag: u32,
        on: bool,
    },
}

#[derive(Default)]
pub struct PeopleUi {
    pub filter: String,
    pub friends_tab: usize,
    /// Object name typed in "Bloquer par nom".
    pub block_name: String,
}

/// Nearby avatars from coarse locations + loaded avatar objects.
pub fn nearby(world: &World) -> Vec<(Uuid, String, Vec3, f32)> {
    let me = world.agent.position;
    let mut out: Vec<(Uuid, String, Vec3, f32)> = Vec::new();
    for (h, list) in &world.coarse {
        let Some(off) = world.region_offset(*h) else {
            continue;
        };
        for (id, p) in list {
            if *id == world.agent_id {
                continue;
            }
            let pos = off + *p;
            let name = world.person_name(id).unwrap_or_else(|| "…".into());
            out.push((*id, name, pos, pos.truncate().distance(me.truncate())));
        }
    }
    for (_, o) in world.objects.iter() {
        if o.is_avatar()
            && o.full_id != world.agent_id
            && !out.iter().any(|e| e.0 == o.full_id)
            && let Some(off) = world.region_offset(o.key.region)
        {
            let pos = off + o.position;
            out.push((o.full_id, world.person_name(&o.full_id).unwrap_or_default(), pos, pos.distance(me)));
        }
    }
    out.sort_by(|a, b| a.3.total_cmp(&b.3));
    out
}

fn distance_color(p: &Palette, d: f32) -> Color32 {
    if d <= 20.0 {
        p.success
    } else if d <= 100.0 {
        p.warn
    } else {
        p.muted
    }
}

pub fn people_window(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    panels: &mut Panels,
    st: &mut PeopleUi,
    // the mini-map renderer (LLNetMap in the Nearby tab)
    map: &mut dyn FnMut(&mut egui::Ui, Vec2),
) -> Vec<PeopleAction> {
    let mut actions = Vec::new();
    let mut open = panels.people;
    Floater::new("people", "Personnes", egui::pos2(8.0, 70.0), Vec2::new(330.0, 420.0))
        .help("Avatars à proximité et liste d'amis")
        .show(ctx, p, &mut open, |ui| {
            let mut tab = panels.people_tab as usize;
            widgets::tabs(
                ui,
                p,
                &mut tab,
                &[
                    ("Près de vous", true),
                    ("Amis", true),
                    ("Groupes", true),
                    ("Récent", false),
                    ("Bloqué", true),
                ],
            );
            panels.people_tab = tab as u8;
            ui.add_space(4.0);
            widgets::search_field(ui, &mut st.filter, "Filtrer", ui.available_width());
            ui.add_space(4.0);
            let filter = st.filter.trim().to_lowercase();
            if panels.people_tab == 0 {
                ui.vertical_centered(|ui| map(ui, Vec2::new(ui.available_width().min(320.0), 150.0)));
                ui.add_space(4.0);
                let list: Vec<_> = nearby(world)
                    .into_iter()
                    .filter(|e| filter.is_empty() || e.1.to_lowercase().contains(&filter))
                    .collect();
                let w = ui.available_width();
                let title = format!("Nom [{}]", list.len());
                widgets::header_row(ui, p, &[(&title, w - 70.0), ("Distance", 70.0)]);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (id, name, _, d) in list.iter().take(200) {
                        ui.horizontal(|ui| {
                            ui.set_min_height(18.0);
                            let r = ui.add(egui::Label::new(RichText::new(name).size(13.0).color(p.ink)).sense(egui::Sense::click()));
                            r.context_menu(|ui| {
                                if ui.button("Voir le profil").clicked() {
                                    actions.push(PeopleAction::Profile(*id));
                                    ui.close();
                                }
                                if ui.button("Envoyer un IM").clicked() {
                                    actions.push(PeopleAction::OpenIm(*id));
                                    ui.close();
                                }
                                if ui.button("Proposer une téléportation").clicked() {
                                    actions.push(PeopleAction::OfferTeleport(*id));
                                    ui.close();
                                }
                                let blocked = world.is_avatar_blocked(id);
                                if ui.button(if blocked { "Débloquer" } else { "Bloquer" }).clicked() {
                                    let legacy = world.legacy_name(id).unwrap_or_else(|| name.clone());
                                    actions.push(PeopleAction::BlockAvatar(*id, legacy, !blocked));
                                    ui.close();
                                }
                            });
                            if r.double_clicked() {
                                actions.push(PeopleAction::OpenIm(*id));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{d:.2}")).size(12.0).strong().color(distance_color(p, *d)));
                            });
                        });
                    }
                });
            } else if panels.people_tab == 2 {
                groups_tab(ui, p, world, &filter, &mut actions);
            } else if panels.people_tab == 4 {
                blocked_tab(ui, p, world, st, &filter, &mut actions);
            } else {
                let friends = world.social.sorted_friends();
                let online = friends.iter().filter(|f| f.online).count();
                let l1 = format!("En ligne ({online})");
                let l2 = format!("Tous ({})", friends.len());
                widgets::tabs(ui, p, &mut st.friends_tab, &[(&l1, true), (&l2, true)]);
                ui.add_space(2.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let mut shown = 0;
                    for f in friends
                        .iter()
                        .filter(|f| st.friends_tab == 1 || f.online)
                        .filter(|f| filter.is_empty() || world.social.name_of(&f.id).to_lowercase().contains(&filter))
                    {
                        shown += 1;
                        ui.horizontal(|ui| {
                            ui.set_min_height(20.0);
                            widgets::status_dot(ui, widgets::online_color(p, f.online));
                            let name = world.social.name_of(&f.id);
                            let col = if f.online { p.indigo_light } else { p.muted };
                            // complete names can be long: cut before the rights
                            // and buttons (full name on hover)
                            let w = (ui.available_width() - 140.0).max(60.0);
                            let r = ui
                                .allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(name).size(13.0).color(col))
                                            .truncate()
                                            .sense(egui::Sense::click()),
                                    )
                                })
                                .inner;
                            if r.double_clicked() {
                                actions.push(PeopleAction::OpenIm(f.id));
                            }
                            r.context_menu(|ui| {
                                if ui.button("Voir le profil").clicked() {
                                    actions.push(PeopleAction::Profile(f.id));
                                    ui.close();
                                }
                                if ui.button("Envoyer un IM").clicked() {
                                    actions.push(PeopleAction::OpenIm(f.id));
                                    ui.close();
                                }
                                if f.online && ui.button("Proposer une téléportation").clicked() {
                                    actions.push(PeopleAction::OfferTeleport(f.id));
                                    ui.close();
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                if f.online
                                    && widgets::flat_button(ui, p, "TP")
                                        .on_hover_text("Proposer une téléportation")
                                        .clicked()
                                {
                                    actions.push(PeopleAction::OfferTeleport(f.id));
                                }
                                if widgets::flat_button(ui, p, "IM").on_hover_text("Message instantané (IM)").clicked() {
                                    actions.push(PeopleAction::OpenIm(f.id));
                                }
                                if widgets::flat_button(ui, p, "i").on_hover_text("Voir le profil").clicked() {
                                    actions.push(PeopleAction::Profile(f.id));
                                }
                                // Rights granted to me: 1 online status, 2 map location, 4 modify objects
                                for (bit, ch, tip) in [
                                    (4, "M", "Je peux modifier ses objets"),
                                    (2, "C", "Je le/la vois sur la carte"),
                                    (1, "E", "Je vois son statut en ligne"),
                                ] {
                                    let on = f.rights_has & bit != 0;
                                    ui.label(RichText::new(ch).size(11.0).color(if on { p.violet_light } else { p.muted_dim }))
                                        .on_hover_text(tip);
                                }
                            });
                        });
                    }
                    if shown == 0 {
                        ui.label(RichText::new("Personne ici pour l'instant.").size(12.0).color(p.muted));
                    }
                });
            }
        });
    panels.people = open;
    actions
}

/// Our groups (LLPanelGroups): chat, block the group's chat.
fn groups_tab(ui: &mut egui::Ui, p: &Palette, world: &World, filter: &str, actions: &mut Vec<PeopleAction>) {
    let groups = world.groups.sorted();
    let w = ui.available_width();
    let title = format!("Groupe [{}]", groups.len());
    widgets::header_row(ui, p, &[(&title, w - 110.0), ("", 110.0)]);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if groups.is_empty() {
            ui.label(RichText::new("Aucun groupe pour l'instant.").size(12.0).color(p.muted));
        }
        for g in groups
            .iter()
            .filter(|g| filter.is_empty() || g.name.to_lowercase().contains(filter))
        {
            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                let blocked = world.mutes.group_chat_muted(&g.id);
                let col = if blocked { p.muted } else { p.ink };
                let name_w = (ui.available_width() - 120.0).max(60.0);
                let r = ui
                    .allocate_ui_with_layout(Vec2::new(name_w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&g.name).size(13.0).color(col))
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    })
                    .inner;
                let r = if blocked { r.on_hover_text("Chat du groupe bloqué") } else { r };
                if r.double_clicked() {
                    actions.push(PeopleAction::GroupChat(g.id));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (label, tip) = if blocked {
                        ("Débloquer", "Recevoir de nouveau le chat de ce groupe")
                    } else {
                        ("Bloquer", "Ne plus recevoir le chat de ce groupe")
                    };
                    if widgets::flat_button(ui, p, label).on_hover_text(tip).clicked() {
                        actions.push(PeopleAction::GroupChatBlocked(g.id, !blocked));
                    }
                    if widgets::flat_button(ui, p, "Chat")
                        .on_hover_text("Ouvrir le chat du groupe")
                        .clicked()
                    {
                        actions.push(PeopleAction::GroupChat(g.id));
                    }
                });
            });
        }
    });
}

/// Block list (LLPanelBlockedList): entries, their blocked properties,
/// unblock, block an object by name.
fn blocked_tab(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut PeopleUi, filter: &str, actions: &mut Vec<PeopleAction>) {
    use crate::world::mutes::LoadState;
    let status = match world.mutes.state {
        LoadState::Initial | LoadState::Requested(_) => Some("Chargement de la liste de blocage…"),
        LoadState::Degraded => Some("Le serveur n'a pas répondu : copie locale de la liste."),
        LoadState::Loaded => None,
    };
    if let Some(s) = status {
        ui.label(RichText::new(s).size(11.5).color(p.muted));
    }
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 130.0).max(60.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut st.block_name)
                .hint_text("Nom exact d'un objet")
                .desired_width(w),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ok = !st.block_name.trim().is_empty();
        if (widgets::flat_button(ui, p, "Bloquer par nom").clicked() || enter) && ok {
            actions.push(PeopleAction::BlockByName(st.block_name.trim().to_owned()));
            st.block_name.clear();
        }
    });
    ui.add_space(4.0);
    let entries = world.mutes.entries();
    let w = ui.available_width();
    let title = format!("Nom [{}]", entries.len());
    widgets::header_row(ui, p, &[(&title, w - 160.0), ("Type", 70.0), ("", 90.0)]);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if entries.is_empty() {
            ui.label(RichText::new("Personne n'est bloqué.").size(12.0).color(p.muted));
        }
        for m in entries
            .iter()
            .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(filter))
        {
            ui.horizontal(|ui| {
                ui.set_min_height(20.0);
                let name = if m.name.is_empty() { "(sans nom)" } else { m.name.as_str() };
                let partial = m.kind != MuteType::ByName && m.flags != 0;
                let name_w = (ui.available_width() - 170.0).max(60.0);
                let r = ui
                    .allocate_ui_with_layout(Vec2::new(name_w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(name).size(13.0).color(if partial { p.muted } else { p.ink }))
                                .truncate()
                                .sense(egui::Sense::click()),
                        )
                    })
                    .inner;
                let r = if partial {
                    r.on_hover_text("Blocage partiel (clic droit pour le détail)")
                } else {
                    r
                };
                if m.kind != MuteType::ByName {
                    r.context_menu(|ui| {
                        if m.kind == MuteType::Agent && ui.button("Voir le profil").clicked() {
                            actions.push(PeopleAction::Profile(m.id));
                            ui.close();
                        }
                        ui.label(RichText::new("Bloquer :").size(11.5).color(p.muted));
                        for (bit, label) in [
                            (flag::TEXT_CHAT, "Chat et IM"),
                            (flag::VOICE_CHAT, "Voix"),
                            (flag::PARTICLES, "Particules"),
                            (flag::OBJECT_SOUNDS, "Sons des objets"),
                        ] {
                            let mut on = m.flags & bit == 0;
                            if ui.checkbox(&mut on, label).changed() {
                                actions.push(PeopleAction::BlockFlag {
                                    id: m.id,
                                    name: m.name.clone(),
                                    kind: m.kind,
                                    flag: bit,
                                    on,
                                });
                            }
                        }
                    });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if widgets::flat_button(ui, p, "Débloquer").clicked() {
                        actions.push(PeopleAction::Unblock(m.id, m.name.clone()));
                    }
                    ui.add_sized(
                        [66.0, 18.0],
                        egui::Label::new(RichText::new(m.kind.label()).size(12.0).color(p.muted)),
                    );
                });
            });
        }
    });
}
