//! The inspector of an object known only by its `objectim` link (an
//! object's IM, a script's message): its name, its owner (resident or
//! group, a link to the profile), where it is (a teleport link), and actions
//! to show its location or block the object or its owner. Port of LLInspectRemoteObject
//! (indra/newview/llinspectremoteobject.cpp, inspect_remote_object.xml,
//! originally LGPL 2.1), in a small window instead of a hover inspector.

use super::context::{CtxAction, request};
use super::widgets::{ActionKind, Floater, action_button_if};
use crate::slapp::RemoteObject;
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};

/// The place of the object's SLURL ("Region/x/y/z"), if it has one.
fn place(o: &RemoteObject) -> Option<crate::slurl::PlaceLink> {
    (!o.slurl.is_empty())
        .then(|| crate::slurl::place_link(&format!("secondlife:///app/teleport/{}", o.slurl)))
        .flatten()
}

/// Shows `shown` until it is closed.
pub fn show(ctx: &egui::Context, p: &Palette, world: &World, shown: &mut Option<RemoteObject>) {
    let Some(o) = shown.clone() else { return };
    let screen = ctx.content_rect();
    let mut open = true;
    let mut close = false;
    let place = place(&o).filter(|place| !world.rlva_private_region(&place.location.region));
    let name = if o.name.is_empty() {
        aurora_i18n::tr!("status-unknown").to_owned()
    } else {
        world.rlva_filter_text(&o.name).into_owned()
    };
    let mut floater = Floater::new(
        "remote_object",
        aurora_i18n::tr!("ui-remote-object-title"),
        egui::pos2(screen.center().x - 320.0, screen.center().y - 95.0),
        Vec2::new(640.0, 190.0),
    );
    floater.min_size = Vec2::new(360.0, 190.0);
    floater.show(ctx, p, &mut open, |ui| {
        let k = super::colors::get();
        let link_color = super::colors::text(k.chat_slurl);
        let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(90.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(90.0);
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                });
                add(ui);
            });
        };
        row(ui, aurora_i18n::tr!("common-object"), &mut |ui| {
            ui.add(egui::Label::new(RichText::new(&name).size(12.0).color(p.ink)).wrap());
        });
        if !o.group_owned && !o.owner.is_nil() {
            // Resolve the canonical mute name even under @shownames, without
            // exposing it in the inspector or storing an anonymous alias.
            world.social.avatar_names.want(&o.owner);
        }
        // the owner: a group's or a resident's profile, "(unknown)" without one
        row(ui, aurora_i18n::tr!("ui-remote-object-owner"), &mut |ui| {
            if o.owner.is_nil() {
                ui.label(RichText::new(aurora_i18n::tr!("status-unknown")).size(12.0).color(p.ink));
                return;
            }
            if let Some(anonymous) = hidden_owner(world, &o) {
                ui.label(RichText::new(anonymous).size(12.0).color(p.ink));
                return;
            }
            let name = if o.group_owned {
                super::chat::app_link_group_name(world, o.owner)
            } else {
                world.social.name_of(&o.owner)
            };
            if ui.add(egui::Link::new(RichText::new(name).size(12.0).color(link_color))).clicked() {
                if o.group_owned {
                    super::group_profile::request_open(ui.ctx(), o.owner);
                } else {
                    super::profile::request_open(ui.ctx(), o.owner);
                }
            }
        });
        // the SLURL as a teleport link
        if let Some(l) = &place {
            row(ui, aurora_i18n::tr!("ui-remote-object-location"), &mut |ui| {
                let r = ui.add(egui::Link::new(RichText::new(&l.label).size(12.0).color(link_color)));
                if r.on_hover_text(aurora_i18n::tr!("ui-chat-click-to-teleport-to-this-place"))
                    .clicked()
                {
                    request(ui.ctx(), CtxAction::TeleportToPlace(l.location.target(), l.location.pos));
                }
            });
        }
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if action_button_if(
                ui,
                p,
                "map",
                aurora_i18n::tr!("ui-remote-object-show-on-map"),
                ActionKind::Flat,
                size,
                place.is_some(),
            )
            .clicked()
                && let Some(l) = &place
            {
                request(ui.ctx(), CtxAction::ShowPlace(l.location.target(), l.location.pos));
                close = true;
            }
            // onClickBlock: disabled once the object is blocked
            let blocked = world.mutes.is_muted_id(&o.id);
            if action_button_if(
                ui,
                p,
                "ban",
                aurora_i18n::tr!("ui-remote-object-block-object"),
                ActionKind::FlatDanger,
                size,
                !o.id.is_nil() && !blocked,
            )
            .clicked()
            {
                request(
                    ui.ctx(),
                    CtxAction::ToggleBlockObject {
                        id: o.id,
                        name: o.name.clone(),
                    },
                );
                close = true;
            }
            if !o.owner.is_nil() && o.owner != world.agent_id {
                let enabled = world.object_owner_block(o.owner, o.group_owned).is_some();
                if action_button_if(
                    ui,
                    p,
                    "ban",
                    aurora_i18n::tr!("ui-remote-object-block-owner"),
                    ActionKind::FlatDanger,
                    size,
                    enabled,
                )
                .clicked()
                {
                    request(
                        ui.ctx(),
                        CtxAction::BlockObjectOwner {
                            owner: o.owner,
                            group_owned: o.group_owned,
                        },
                    );
                    close = true;
                }
            }
            if action_button_if(ui, p, "x", aurora_i18n::tr!("common-close"), ActionKind::Flat, size, true).clicked() {
                close = true;
            }
        });
    });
    if !open || close {
        *shown = None;
    }
}

#[cfg(test)]
mod tests;

/// The owner's name in its place under RLVa `@shownames`: a resident owner
/// is shown anonymous, with no profile link (the Legacy Viewer's objectim
/// link carries `rlv_shownames` and LLInspectRemoteObject shows
/// `rlvanonym`). A group owner stays named, as there. Stricter than the
/// Legacy Viewer, which hides only an owner who is nearby.
pub(crate) fn hidden_owner(world: &World, o: &crate::slapp::RemoteObject) -> Option<String> {
    (!o.owner.is_nil() && !o.group_owned && !world.rlva_can_name(o.owner))
        .then(|| crate::world::rlva::privacy::anonymous(o.owner).display_name.clone())
}
