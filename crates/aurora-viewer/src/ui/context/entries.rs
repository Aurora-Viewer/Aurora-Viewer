//! Shared world menu entries and permissions, following Firestorm menu_*.xml.

use super::{CtxAction, Facts, Target};
use crate::build::script_queue::QueueKind;
use crate::report_abuse::ReportTarget;
use crate::world::objects::ObjKey;
use glam::Vec3;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Entry {
    pub icon: &'static str,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub kind: Kind,
}

#[derive(Clone)]
pub(super) enum Kind {
    Action(CtxAction),
    Submenu(Vec<Entry>),
    Unavailable,
    Inactive,
    Separator,
    Info,
}

#[derive(Default)]
struct Builder(Vec<Entry>);

impl Builder {
    fn item(&mut self, icon: &'static str, label: &str, action: CtxAction) {
        self.item_if(icon, label, true, action);
    }

    fn item_if(&mut self, icon: &'static str, label: &str, enabled: bool, action: CtxAction) {
        self.0.push(Entry {
            icon,
            label: label.into(),
            enabled,
            checked: None,
            kind: Kind::Action(action),
        });
    }

    fn check(&mut self, icon: &'static str, label: &str, checked: bool, enabled: bool, action: CtxAction) {
        self.item_if(icon, label, enabled, action);
        if let Some(entry) = self.0.last_mut() {
            entry.checked = Some(checked);
        }
    }

    fn todo(&mut self, icon: &'static str, label: &str) {
        self.0.push(Entry {
            icon,
            label: label.into(),
            enabled: false,
            checked: None,
            kind: Kind::Unavailable,
        });
    }

    fn disabled(&mut self, icon: &'static str, label: &str) {
        self.0.push(Entry {
            icon,
            label: label.into(),
            enabled: false,
            checked: None,
            kind: Kind::Inactive,
        });
    }

    fn separator(&mut self) {
        self.0.push(Entry {
            icon: "",
            label: String::new(),
            enabled: false,
            checked: None,
            kind: Kind::Separator,
        });
    }

    fn info(&mut self, icon: &'static str, label: &str) {
        self.0.push(Entry {
            icon,
            label: label.into(),
            enabled: false,
            checked: None,
            kind: Kind::Info,
        });
    }

    fn submenu(&mut self, icon: &'static str, label: &str, enabled: bool, body: impl FnOnce(&mut Builder)) {
        let mut children = Builder::default();
        body(&mut children);
        self.0.push(Entry {
            icon,
            label: label.into(),
            enabled,
            checked: None,
            kind: Kind::Submenu(children.0),
        });
    }
}

pub(super) fn world_menu(target: &Target, f: &Facts, point: Vec3) -> Vec<Entry> {
    let mut entries = Builder::default();
    match target {
        Target::Object { key, full_id, offset } => object_menu(&mut entries, f, *key, *full_id, *offset, point),
        Target::Attachment { key } => attachment_self_menu(&mut entries, f, *key),
        Target::Avatar { id, own: true, .. } => self_menu(&mut entries, f, *id),
        Target::Avatar {
            id,
            attachment: Some(attachment),
            ..
        } => attachment_other_menu(&mut entries, f, *id, *attachment, point),
        Target::Avatar { id, .. } => avatar_menu(&mut entries, f, *id, point),
        Target::Ground => land_menu(&mut entries, f, point),
    }
    entries.0
}

pub(super) fn draw_list(ui: &mut egui::Ui, p: &crate::theme::Palette, entries: &[Entry]) {
    use super::menu;
    for entry in entries {
        match &entry.kind {
            Kind::Action(action) => {
                let clicked = match entry.checked {
                    Some(checked) => menu::check(ui, p, entry.icon, &entry.label, checked, entry.enabled),
                    None => menu::item_if(ui, p, entry.icon, &entry.label, entry.enabled),
                };
                if clicked {
                    super::request(ui.ctx(), action.clone());
                }
            }
            Kind::Submenu(children) => menu::submenu(ui, p, entry.icon, &entry.label, entry.enabled, |ui| draw_list(ui, p, children)),
            Kind::Unavailable => menu::todo(ui, p, entry.icon, &entry.label),
            Kind::Inactive => {
                menu::item_if(ui, p, entry.icon, &entry.label, false);
            }
            Kind::Separator => menu::separator(ui, p),
            Kind::Info => menu::info(ui, p, entry.icon, &entry.label),
        }
    }
}

/// menu_object.xml
#[allow(clippy::too_many_arguments)]
fn object_menu(ui: &mut Builder, f: &Facts, key: ObjKey, id: Uuid, offset: Vec3, point: Vec3) {
    let o = &f.object;
    ui.item_if("click-filled", touch_label(o), o.touch, CtxAction::Touch(key.local_id));
    ui.item("pencil-filled", aurora_i18n::tr!("common-edit"), CtxAction::Edit(id));
    ui.todo("paint-filled", aurora_i18n::tr!("ui-context-edit-pbr-material"));
    // Object.Build: the Create tool (LLToolCompCreate), like the land menu
    ui.item("box", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item_if("package", aurora_i18n::tr!("common-open"), o.open, CtxAction::Open(id));
    ui.separator();
    // enable_object_sit: the object's sit text when it has one
    let sit = if o.sit_name.is_empty() {
        aurora_i18n::tr!("ui-context-sit-here")
    } else {
        &o.sit_name
    };
    ui.item_if("armchair", sit, o.sit, CtxAction::Sit { target: id, offset });
    ui.separator();
    ui.item_if("man-filled", aurora_i18n::tr!("common-stand-up"), o.stand, CtxAction::StandUp);
    ui.separator();
    ui.submenu("box", aurora_i18n::tr!("common-object"), true, |ui| {
        ui.todo("info-circle-filled", aurora_i18n::tr!("common-profile"));
        ui.item("zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2"), CtxAction::Inspect(id));
        ui.item("code", aurora_i18n::tr!("ui-context-script-information"), CtxAction::ScriptInfo(id));
        ui.separator();
        // two objects or more: a right click on a selected one keeps them;
        // one alone has nothing to link to
        if o.roots > 1 {
            ui.item_if("link", aurora_i18n::tr!("ui-context-link"), o.link, CtxAction::Link(id));
        } else {
            ui.disabled("link", aurora_i18n::tr!("ui-context-link"));
        }
        ui.item_if("unlink", aurora_i18n::tr!("ui-context-unlink"), o.unlink, CtxAction::Unlink(id));
        ui.check(
            "cube-3d-sphere",
            aurora_i18n::tr!("ui-context-edit-linked-parts"),
            f.edit_linked,
            true,
            CtxAction::EditLinkedParts(!f.edit_linked),
        );
        ui.separator();
        // Tools.SelectedScriptAction (EditableSelected)
        for (icon, label, action) in [
            ("refresh", aurora_i18n::tr!("ui-context-reset-scripts"), QueueKind::Reset),
            ("player-play-filled", aurora_i18n::tr!("ui-context-run-scripts"), QueueKind::Run),
            ("player-stop-filled", aurora_i18n::tr!("ui-context-stop-scripts"), QueueKind::Stop),
            ("trash-filled", aurora_i18n::tr!("ui-context-remove-scripts"), QueueKind::Delete),
        ] {
            ui.item_if(icon, label, o.scripts, CtxAction::Scripts { id, action });
        }
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    // Pathfinding.Linksets.Select / Characters.Select: the pathfinding
    // windows are not in Aurora yet
    ui.todo("list-details-filled", aurora_i18n::tr!("ui-context-show-in-region-objects"));
    ui.todo("walk", aurora_i18n::tr!("ui-context-show-in-characters"));
    ui.separator();
    put_on_submenu(ui, f, id);
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        // Object.ReportAbuse: showFromObject on the object
        ui.item(
            "flag-filled",
            aurora_i18n::tr!("common-report"),
            CtxAction::ReportAbuse(ReportTarget::Object(id)),
        );
        let (icon, label) = if o.blocked {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("ban", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlockObject { id, name: o.name.clone() });
    });
    ui.separator();
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshTextures(id),
    );
    derender_items(ui, id, aurora_i18n::tr!("ui-context-derender"), true);
    // Object.EnableReturn: any object that is not worn
    ui.item("arrow-back-up", aurora_i18n::tr!("ui-context-return"), CtxAction::Return(id));
    if o.roots > 1 {
        take_multiple_submenu(ui, o, id);
    } else {
        ui.item_if("hand-grab", aurora_i18n::tr!("ui-context-take"), o.take, CtxAction::Take(id));
        ui.item_if(
            "copy-filled",
            aurora_i18n::tr!("ui-context-take-copy"),
            o.take_copy,
            CtxAction::TakeCopy(id),
        );
    }
    ui.item_if("coin-filled", aurora_i18n::tr!("common-pay"), o.pay, CtxAction::Pay(id));
    ui.item_if("shopping-cart-filled", aurora_i18n::tr!("common-buy"), o.buy, CtxAction::Buy(id));
    ui.todo("device-floppy-filled", aurora_i18n::tr!("common-save-as"));
    ui.separator();
    ui.item_if("trash-filled", aurora_i18n::tr!("common-delete"), o.delete, CtxAction::Delete(id));
    ui.separator();
    particle_owner_item(ui, f, aurora_i18n::tr!("ui-context-ignore-particle-owner"));
    // Avatar.ResetSkeleton, shown for an animated object (its skeleton)
    if let Some(skeleton) = o.animesh {
        ui.item(
            "bone-filled",
            aurora_i18n::tr!("ui-context-reset-skeleton"),
            CtxAction::ResetSkeleton {
                avatar: skeleton,
                animations: false,
            },
        );
    }
}

/// "Touch", or the object's touch text (enable_object_touch).
fn touch_label(o: &super::ObjectFacts) -> &str {
    if o.touch_name.is_empty() {
        aurora_i18n::tr!("ui-context-touch")
    } else {
        &o.touch_name
    }
}

/// "Block particle owner" (Particle.Mute, EnableMuteParticle): the owner
/// of the particle the right click landed on, when it is not us.
fn particle_owner_item(ui: &mut Builder, f: &Facts, label: &str) {
    ui.item_if(
        "sparkles-filled",
        label,
        f.particle_owner.is_some(),
        CtxAction::BlockParticleOwner(f.particle_owner.unwrap_or_default()),
    );
}

/// "Put on" (menu_object.xml Put On): Wear, Add, and the attachment points
/// by body part (LLVOAvatarSelf::buildMenus: groups 1 to 8 in submenus by
/// pie slice, 9 the HUD), all enabled by object_is_wearable.
fn put_on_submenu(ui: &mut Builder, f: &Facts, id: Uuid) {
    let wear = f.object.wear;
    ui.submenu("shirt-filled", aurora_i18n::tr!("ui-context-wear"), true, |ui| {
        ui.item_if(
            "shirt-filled",
            aurora_i18n::tr!("common-wear"),
            wear,
            CtxAction::Wear { id, point: 0, add: false },
        );
        ui.item_if(
            "circle-plus-filled",
            aurora_i18n::tr!("common-add"),
            wear,
            CtxAction::Wear { id, point: 0, add: true },
        );
        let point = |c: &super::AttachChoice| CtxAction::Wear {
            id,
            point: c.id,
            add: true,
        };
        ui.submenu("paperclip", aurora_i18n::tr!("common-attach-to"), true, |ui| {
            for (group, label) in [
                (1, aurora_i18n::tr!("ui-context-body-right-arm")),
                (2, aurora_i18n::tr!("ui-context-body-head")),
                (3, aurora_i18n::tr!("ui-context-body-left-arm")),
                (5, aurora_i18n::tr!("ui-context-body-left-leg")),
                (6, aurora_i18n::tr!("ui-context-body-torso")),
                (7, aurora_i18n::tr!("ui-context-body-right-leg")),
                (8, aurora_i18n::tr!("ui-context-body-enhanced-skeleton")),
            ] {
                ui.submenu("man-filled", label, true, |ui| {
                    for c in f.attach_points.iter().filter(|c| c.group == group) {
                        ui.item_if("man-filled", &c.label, wear, point(c));
                    }
                });
            }
        });
        ui.submenu("device-desktop-filled", aurora_i18n::tr!("common-attach-to-hud"), true, |ui| {
            for c in f.attach_points.iter().filter(|c| c.group == 9) {
                ui.item_if("device-desktop-filled", &c.label, wear, point(c));
            }
        });
    });
}

/// "Take multiple" (Object.VisibleTakeMultiple: two roots or more): as
/// one item or one per root, taken or copied.
fn take_multiple_submenu(ui: &mut Builder, o: &super::ObjectFacts, id: Uuid) {
    ui.submenu("hand-grab", aurora_i18n::tr!("ui-context-take-multiple"), true, |ui| {
        ui.item_if(
            "hand-grab",
            aurora_i18n::tr!("ui-context-take-combined"),
            o.take,
            CtxAction::Take(id),
        );
        ui.item_if(
            "copy-filled",
            aurora_i18n::tr!("ui-context-take-copy-combined"),
            o.take_copy,
            CtxAction::TakeCopy(id),
        );
        ui.item_if(
            "hand-grab",
            aurora_i18n::tr!("ui-context-take-separate"),
            o.take,
            CtxAction::TakeSeparate { id, copy: false },
        );
        ui.item_if(
            "copy-filled",
            aurora_i18n::tr!("ui-context-take-copy-separate"),
            o.take_copy,
            CtxAction::TakeSeparate { id, copy: true },
        );
    });
}

/// menu_attachment_self.xml
fn attachment_self_menu(ui: &mut Builder, f: &Facts, key: ObjKey) {
    if let Some(scale) = f.object.local_hud_scale {
        ui.info(
            "zoom-filled",
            &aurora_i18n::tr!("hud-local-scale", percent = format!("{:.0}", scale * 100.0)),
        );
        ui.item("restore", aurora_i18n::tr!("hud-local-scale-reset"), CtxAction::ResetHudScale(key));
        ui.item_if(
            "backspace-filled",
            aurora_i18n::tr!("hud-local-scale-forget"),
            f.object.saved_hud_scale,
            CtxAction::ForgetHudScale(key),
        );
        ui.separator();
    }
    ui.item_if(
        "click-filled",
        touch_label(&f.object),
        f.object.touch,
        CtxAction::Touch(key.local_id),
    );
    ui.item_if(
        "briefcase-filled",
        aurora_i18n::tr!("ui-context-show-in-inventory"),
        f.object.attachment_item.is_some(),
        CtxAction::AttachmentInventory(f.object.attachment_item.unwrap_or_default()),
    );
    // Object.SetFavorite Add / Remove, the one that applies
    if let (Some(item), Some(favorite)) = (f.object.attachment_item, f.object.favorite) {
        let label = if favorite {
            aurora_i18n::tr!("common-remove-from-favorites")
        } else {
            aurora_i18n::tr!("common-add-to-favorites")
        };
        ui.item("star-filled", label, CtxAction::AttachmentFavorite(item));
    }
    ui.item_if(
        "pencil-filled",
        aurora_i18n::tr!("common-edit"),
        f.object.edit,
        CtxAction::EditAttachment(key),
    );
    ui.todo("paint-filled", aurora_i18n::tr!("ui-context-edit-pbr-material"));
    ui.item_if(
        "paperclip",
        aurora_i18n::tr!("common-detach"),
        f.object.attachment_item.is_some(),
        CtxAction::Detach(key),
    );
    save_as_submenu(ui);
    ui.separator();
    sit_stand(ui, f);
    appearance_submenu(ui, f);
    take_off_submenu(ui);
    ui.separator();
    // Self.ToggleSitStand
    ui.item(
        "armchair",
        aurora_i18n::tr!("ui-context-sit-stand"),
        if f.seated { CtxAction::StandUp } else { CtxAction::SitGround },
    );
    // enable_fly_land: flying already, or not sitting (LLAgent::enableFlying)
    ui.item_if(
        "plane-departure-filled",
        aurora_i18n::tr!("extra-fly-land-8"),
        f.flying || !f.seated,
        CtxAction::ToggleFly,
    );
    ui.item(
        "player-stop-filled",
        aurora_i18n::tr!("ui-context-stop-animations"),
        CtxAction::StopAnimations,
    );
    ui.separator();
    community_submenu(ui, f.me);
    ui.todo("info-circle-filled", aurora_i18n::tr!("ui-context-object-profile"));
    ui.item_if(
        "zoom-scan-filled",
        aurora_i18n::tr!("ui-context-inspect-2"),
        !f.object.id.is_nil(),
        CtxAction::Inspect(f.object.id),
    );
    ui.item_if(
        "code",
        aurora_i18n::tr!("ui-context-script-information"),
        !f.object.id.is_nil(),
        CtxAction::ScriptInfo(f.object.id),
    );
    derender_items(
        ui,
        f.object.id,
        aurora_i18n::tr!("ui-context-derender-temporarily"),
        !f.object.id.is_nil(),
    );
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    ui.item_if(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        !f.object.id.is_nil(),
        CtxAction::RefreshTextures(f.object.id),
    );
    ui.separator();
    // Attachment.Drop: on the ground, where we may build
    ui.item_if(
        "arrow-bar-to-down",
        aurora_i18n::tr!("ui-context-drop"),
        f.object.drop,
        CtxAction::Drop(key),
    );
    ui.separator();
    particle_owner_item(ui, f, aurora_i18n::tr!("ui-context-block-particle-owner"));
}

/// menu_avatar_other.xml
fn avatar_menu(ui: &mut Builder, f: &Facts, id: Uuid, point: Vec3) {
    ui.item("user-filled", aurora_i18n::tr!("common-view-profile"), CtxAction::Profile(id));
    ui.item_if(
        "user-plus",
        aurora_i18n::tr!("common-become-friends"),
        !f.friend,
        CtxAction::AddFriend(id),
    );
    contact_set_entries(ui, f, id);
    ui.item("message-circle-filled", aurora_i18n::tr!("ui-context-send-im"), CtxAction::Im(id));
    // Avatar.GiveCard (enabled like RLV.CanShowName: always here)
    ui.item(
        "id-filled",
        aurora_i18n::tr!("ui-context-give-my-calling-card"),
        CtxAction::GiveCallingCard(id),
    );
    ui.item_if("phone-filled", aurora_i18n::tr!("common-voice-call"), f.voice, CtxAction::Call(id));
    // Avatar.InviteToGroup (enabled like RLV.CanShowName: always here)
    ui.item(
        "users-plus",
        aurora_i18n::tr!("ui-context-invite-to-group"),
        CtxAction::InviteToGroup(id),
    );
    ui.item(
        "send-filled",
        aurora_i18n::tr!("common-offer-teleport"),
        CtxAction::OfferTeleport(id),
    );
    ui.separator();
    ui.todo("eye-filled", aurora_i18n::tr!("common-look-toward-avatar"));
    ui.separator();
    reset_skeleton_items(ui, id);
    ui.todo("restore", aurora_i18n::tr!("ui-context-reset-mesh-lod-2"));
    ui.separator();
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        let (icon, label) = if f.blocked {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("ban", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlock(id));
        // Avatar.ReportAbuse: showFromObject on the avatar
        ui.item(
            "flag-filled",
            aurora_i18n::tr!("common-report"),
            CtxAction::ReportAbuse(ReportTarget::Object(id)),
        );
        ui.todo("snowflake", aurora_i18n::tr!("ui-context-freeze"));
        ui.todo("player-eject-filled", aurora_i18n::tr!("ui-context-eject"));
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.item("coin-filled", aurora_i18n::tr!("common-pay"), CtxAction::PayResident(id));
    ui.separator();
    // Firestorm "Always Display Full Detail / Never Display Full Detail / Remove From Display Exceptions"
    if let Some((c, silhouette)) = f.complexity {
        let text = aurora_i18n::tr!(
            "ui-context-complexity",
            value0 = crate::ui::hud::group_digits(c).to_string(),
            value1 = (if silhouette { " (silhouette)" } else { "" }).to_string()
        );
        ui.info("gauge-filled", &text);
    }
    for (mode, icon, label) in [
        (2u8, "user-scan", aurora_i18n::tr!("ui-context-always-show-full-detail")),
        (1, "ghost-filled", aurora_i18n::tr!("ui-context-never-fully-render")),
        (
            0,
            "adjustments-filled",
            aurora_i18n::tr!("ui-context-display-according-to-your-preferences"),
        ),
    ] {
        ui.check(icon, label, f.render == mode, true, CtxAction::SetRender(id, mode));
    }
    // the Inspect window on everything the avatar wears (each worn
    // object's own menu inspects that one, menu_attachment_other.xml)
    ui.item_if(
        "zoom-scan-filled",
        aurora_i18n::tr!("ui-context-inspect-2"),
        f.worn,
        CtxAction::InspectAvatar(id),
    );
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    derender_items(ui, id, aurora_i18n::tr!("ui-context-derender-temporarily"), true);
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshAvatarTextures(id),
    );
    ui.separator();
    // only the particle flag of their block list entry, checked while it
    // is on (the block list's "Block particles", LLMute::flagParticles);
    // the Legacy Viewer's "Block Particle Owner" needs a clicked particle
    ui.check(
        "sparkles-filled",
        aurora_i18n::tr!("common-block-particles"),
        f.particles_blocked,
        true,
        CtxAction::ToggleParticleMute(id),
    );
}

/// menu_attachment_other.xml: the avatar's entries, then those of the
/// attachment clicked, in Firestorm's order. Entries it hides unless they
/// apply (Dump XML, Block Particle Owner, Freeze / Eject without the
/// rights, Remove From Display Exceptions without an exception) are left
/// out the same way.
fn attachment_other_menu(ui: &mut Builder, f: &Facts, id: Uuid, attachment: Uuid, point: Vec3) {
    ui.item("user-filled", aurora_i18n::tr!("common-view-profile"), CtxAction::Profile(id));
    ui.item_if(
        "user-plus",
        aurora_i18n::tr!("common-become-friends"),
        !f.friend,
        CtxAction::AddFriend(id),
    );
    contact_set_entries(ui, f, id);
    ui.item("message-circle-filled", aurora_i18n::tr!("ui-context-send-im"), CtxAction::Im(id));
    // Avatar.GiveCard (enabled like RLV.CanShowName: always here)
    ui.item(
        "id-filled",
        aurora_i18n::tr!("ui-context-give-my-calling-card"),
        CtxAction::GiveCallingCard(id),
    );
    ui.item_if("phone-filled", aurora_i18n::tr!("common-voice-call"), f.voice, CtxAction::Call(id));
    // Avatar.InviteToGroup (enabled like RLV.CanShowName: always here)
    ui.item(
        "users-plus",
        aurora_i18n::tr!("ui-context-invite-to-group"),
        CtxAction::InviteToGroup(id),
    );
    ui.todo("eye-filled", aurora_i18n::tr!("common-look-toward-avatar"));
    ui.separator();
    reset_skeleton_items(ui, id);
    ui.todo("restore", aurora_i18n::tr!("ui-context-reset-mesh-lod-2"));
    ui.separator();
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        let (icon, label) = if f.blocked {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("ban", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlock(id));
        ui.item(
            "flag-filled",
            aurora_i18n::tr!("common-report"),
            CtxAction::ReportAbuse(ReportTarget::Object(id)),
        );
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    // Avatar.Pay: the wearer is paid
    ui.item("coin-filled", aurora_i18n::tr!("common-pay"), CtxAction::PayResident(id));
    ui.separator();
    ui.todo("info-circle-filled", aurora_i18n::tr!("ui-context-object-profile"));
    ui.separator();
    // Avatar.SetImpostorMode 2 / 1, and 0 once there is an exception
    ui.check(
        "user-scan",
        aurora_i18n::tr!("ui-context-always-show-full-detail"),
        f.render == 2,
        true,
        CtxAction::SetRender(id, 2),
    );
    ui.check(
        "ghost-filled",
        aurora_i18n::tr!("ui-context-never-fully-render"),
        f.render == 1,
        true,
        CtxAction::SetRender(id, 1),
    );
    if f.render != 0 {
        ui.item(
            "adjustments-filled",
            aurora_i18n::tr!("ui-context-remove-from-display-exceptions"),
            CtxAction::SetRender(id, 0),
        );
    }
    ui.item(
        "zoom-scan-filled",
        aurora_i18n::tr!("ui-context-inspect-2"),
        CtxAction::Inspect(attachment),
    );
    ui.item(
        "code",
        aurora_i18n::tr!("ui-context-script-info"),
        CtxAction::ScriptInfo(attachment),
    );
    // the worn object, not its wearer (the selection's root)
    derender_items(ui, attachment, aurora_i18n::tr!("ui-context-derender-temporarily"), true);
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    // the attachment's textures (Object.TexRefresh), not the avatar's bakes
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshTextures(attachment),
    );
    save_as_submenu(ui);
}

/// Object.Derender / Object.DerenderPermanent (derenderObject on the
/// selection's root: the linkset, the worn object or the avatar), enabled
/// like the Legacy Viewer's (Object.EnableDerender: always without RLVa).
fn derender_items(ui: &mut Builder, id: Uuid, temporary: &str, enabled: bool) {
    ui.item_if("eye-off", temporary, enabled, CtxAction::Derender { id, permanent: false });
    ui.item_if(
        "eye-closed",
        aurora_i18n::tr!("ui-context-derender-blacklist"),
        enabled,
        CtxAction::Derender { id, permanent: true },
    );
}

/// "Save as" (Export Menu): the export has its own task and
/// rules of rights; its two entries wait for it.
fn save_as_submenu(ui: &mut Builder) {
    ui.submenu("device-floppy-filled", aurora_i18n::tr!("common-save-as"), true, |ui| {
        ui.todo("device-floppy-filled", aurora_i18n::tr!("ui-context-save-backup"));
        ui.todo("box", aurora_i18n::tr!("ui-context-save-collada"));
    });
}

/// menu_avatar_self.xml
fn self_menu(ui: &mut Builder, f: &Facts, id: Uuid) {
    sit_stand(ui, f);
    take_off_submenu(ui);
    appearance_submenu(ui, f);
    community_submenu(ui, id);
    // not in Firestorm's menu: kept from Aurora's (fly is in menu_attachment_self.xml)
    ui.item(
        if f.flying {
            "plane-arrival-filled"
        } else {
            "plane-departure-filled"
        },
        if f.flying {
            aurora_i18n::tr!("ui-context-land")
        } else {
            aurora_i18n::tr!("common-fly")
        },
        CtxAction::ToggleFly,
    );
    ui.item("camera-filled", aurora_i18n::tr!("ui-context-reset-camera"), CtxAction::ResetCamera);
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshAvatarTextures(id),
    );
}

/// "Sit" (on the ground, Self.SitDown) and "Stand up".
fn sit_stand(ui: &mut Builder, f: &Facts) {
    ui.item_if("armchair", aurora_i18n::tr!("status-sit"), !f.seated, CtxAction::SitGround);
    ui.item_if("man-filled", aurora_i18n::tr!("common-stand-up"), f.seated, CtxAction::StandUp);
}

fn take_off_submenu(ui: &mut Builder) {
    ui.submenu("shirt-filled", aurora_i18n::tr!("common-remove"), true, |ui| {
        ui.todo("shirt-filled", aurora_i18n::tr!("common-clothing"));
        ui.todo("device-desktop-filled", "HUD");
        ui.todo("paperclip", aurora_i18n::tr!("common-detach"));
        ui.todo("circle-x-filled", aurora_i18n::tr!("ui-context-detach-all"));
    });
}

fn appearance_submenu(ui: &mut Builder, f: &Facts) {
    ui.submenu("man-filled", aurora_i18n::tr!("common-appearance"), true, |ui| {
        ui.item(
            "shirt-filled",
            aurora_i18n::tr!("common-current-outfit"),
            CtxAction::OpenAppearance { tab: 2, editing: false },
        );
        ui.item(
            "hanger-2-filled",
            aurora_i18n::tr!("ui-context-change-outfit"),
            CtxAction::OpenAppearance { tab: 0, editing: false },
        );
        ui.item_if(
            "man-filled",
            aurora_i18n::tr!("ui-context-edit-shape"),
            f.worn_shape.is_some(),
            CtxAction::EditWorn(f.worn_shape.unwrap_or_default()),
        );
        ui.item(
            "pencil-filled",
            aurora_i18n::tr!("ui-context-edit-outfit"),
            CtxAction::OpenAppearance { tab: 2, editing: true },
        );
        ui.item_if(
            "arrows-vertical",
            aurora_i18n::tr!("ui-context-hover-height"),
            f.hover_height,
            CtxAction::HoverHeight,
        );
        ui.separator();
        reset_skeleton_items(ui, f.me);
        ui.todo("restore", aurora_i18n::tr!("ui-context-reset-mesh-lod"));
        // Avatar › Avatar Health › "Force Appearance Update (Rebake)" in
        // Firestorm's menu bar, next to the resets here (Ctrl+Alt+R)
        ui.item("reload", aurora_i18n::tr!("ui-context-rebake"), CtxAction::Rebake);
    });
}

/// "Add to contact set" (Avatar.AddToContactSet): the sets of
/// FSFloaterAddToContactSet's list in a submenu, then its "+"
/// (AddNewContactSet), as `contact_set_submenu` of the lists.
fn contact_set_entries(ui: &mut Builder, f: &Facts, id: Uuid) {
    ui.submenu("users-group", aurora_i18n::tr!("ui-context-add-to-contact-set"), true, |ui| {
        if f.contact_sets.is_empty() {
            ui.disabled("users-group", aurora_i18n::tr!("ui-contacts-no-contact-sets"));
        }
        for set in &f.contact_sets {
            let add = crate::ui::contacts::SetEdit::Add {
                set: set.clone(),
                ids: vec![id],
            };
            ui.item("users-group", set, CtxAction::ContactSet(add));
        }
        ui.separator();
        ui.item(
            "plus",
            aurora_i18n::tr!("ui-context-new-contact-set"),
            CtxAction::NewContactSet(vec![id]),
        );
    });
}

/// Avatar.ResetSkeleton / Avatar.ResetSkeletonAndAnimations: enabled for
/// any avatar (LLAvatarEnableResetSkeleton), ours included.
fn reset_skeleton_items(ui: &mut Builder, avatar: Uuid) {
    ui.item(
        "bone-filled",
        aurora_i18n::tr!("ui-context-reset-skeleton"),
        CtxAction::ResetSkeleton { avatar, animations: false },
    );
    ui.item(
        "run",
        aurora_i18n::tr!("ui-context-reset-skeleton-and-animations"),
        CtxAction::ResetSkeleton { avatar, animations: true },
    );
}

fn community_submenu(ui: &mut Builder, me: Uuid) {
    ui.submenu("users-group", aurora_i18n::tr!("ui-context-community"), true, |ui| {
        ui.item("users", aurora_i18n::tr!("common-friends"), CtxAction::OpenPeople(1));
        ui.item("users-group", aurora_i18n::tr!("common-groups"), CtxAction::OpenPeople(2));
        ui.item("user-filled", aurora_i18n::tr!("common-profile"), CtxAction::Profile(me));
        ui.item("id-filled", aurora_i18n::tr!("ui-context-display-name"), CtxAction::DisplayName);
    });
}

/// menu_land.xml
fn land_menu(ui: &mut Builder, f: &Facts, point: Vec3) {
    ui.item(
        "info-circle-filled",
        aurora_i18n::tr!("common-about-land"),
        CtxAction::AboutLand(point),
    );
    // Land.Sit walks there first (autopilot): not in Aurora yet
    ui.todo("armchair", aurora_i18n::tr!("ui-context-sit-here"));
    if f.seated {
        ui.item("man-filled", aurora_i18n::tr!("common-stand-up"), CtxAction::StandUp);
    }
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.separator();
    // Firestorm greys it out when the parcel is not for sale to us; the
    // menu does not know the parcel before it is selected, so the window
    // says why instead
    ui.item(
        "shopping-cart-filled",
        aurora_i18n::tr!("ui-context-buy-this-land"),
        CtxAction::BuyLand(point),
    );
    ui.todo("ticket-filled", aurora_i18n::tr!("ui-context-buy-access"));
    ui.separator();
    ui.item("box", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item("mountain-filled", aurora_i18n::tr!("ui-context-edit-land"), CtxAction::EditTerrain);
    ui.separator();
    particle_owner_item(ui, f, aurora_i18n::tr!("ui-context-ignore-particle-owner"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Labels of a menu, `-` for a separator, `>` after a submenu.
    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match &e.kind {
                Kind::Separator => "-".to_string(),
                Kind::Submenu(_) => format!("{}>", e.label),
                _ => e.label.clone(),
            })
            .collect()
    }

    fn tr(keys: &[&'static str]) -> Vec<String> {
        keys.iter()
            .map(|k| match *k {
                "-" => "-".to_string(),
                k if k.ends_with('>') => format!("{}>", aurora_i18n::text(k.trim_end_matches('>'))),
                k => aurora_i18n::text(k).to_string(),
            })
            .collect()
    }

    #[test]
    fn attachment_of_another_avatar_has_firestorms_entries_in_order() {
        // menu_attachment_other.xml, without the entries it hides
        let target = Target::Avatar {
            id: Uuid::from_u128(7),
            own: false,
            attachment: Some(Uuid::from_u128(8)),
        };
        let mut f = Facts::default();
        let expected = |exception: bool| {
            let mut keys = vec![
                "common-view-profile",
                "common-become-friends",
                "ui-context-add-to-contact-set>",
                "ui-context-send-im",
                "ui-context-give-my-calling-card",
                "common-voice-call",
                "ui-context-invite-to-group",
                "common-look-toward-avatar",
                "-",
                "ui-context-reset-skeleton",
                "ui-context-reset-skeleton-and-animations",
                "ui-context-reset-mesh-lod-2",
                "-",
                "ui-context-disturbance>",
                "common-zoom-2",
                "common-pay",
                "-",
                "ui-context-object-profile",
                "-",
                "ui-context-always-show-full-detail",
                "ui-context-never-fully-render",
            ];
            if exception {
                keys.push("ui-context-remove-from-display-exceptions");
            }
            keys.extend([
                "ui-context-inspect-2",
                "ui-context-script-info",
                "ui-context-derender-temporarily",
                "ui-context-derender-blacklist",
                "ui-context-show-textures",
                "ui-context-refresh-textures",
                "common-save-as>",
            ]);
            tr(&keys)
        };
        assert_eq!(labels(&world_menu(&target, &f, Vec3::ZERO)), expected(false));
        // "Remove from display exceptions" only with an exception
        f.render = 2;
        assert_eq!(labels(&world_menu(&target, &f, Vec3::ZERO)), expected(true));
        // the avatar itself keeps menu_avatar_other.xml
        let avatar = Target::Avatar {
            id: Uuid::from_u128(7),
            own: false,
            attachment: None,
        };
        assert_ne!(labels(&world_menu(&avatar, &f, Vec3::ZERO)), expected(true));
    }

    #[test]
    fn inspect_and_derender_act_on_the_avatar_or_on_the_worn_object() {
        let find = |entries: Vec<Entry>, key: &'static str, permanent: bool| {
            entries
                .into_iter()
                .find(|e| {
                    e.label == aurora_i18n::text(key)
                        && match &e.kind {
                            Kind::Action(CtxAction::Derender { permanent: p, .. }) => *p == permanent,
                            _ => true,
                        }
                })
                .expect("entry")
        };
        let (avatar, worn) = (Uuid::from_u128(7), Uuid::from_u128(8));
        let on_avatar = Target::Avatar {
            id: avatar,
            own: false,
            attachment: None,
        };
        // "Inspect" lists what the avatar wears: greyed when it wears nothing in view
        let inspect = |f: &Facts| find(world_menu(&on_avatar, f, Vec3::ZERO), "ui-context-inspect-2", false);
        assert!(!inspect(&Facts::default()).enabled);
        let wearing = Facts {
            worn: true,
            ..Facts::default()
        };
        assert!(matches!(inspect(&wearing).kind, Kind::Action(CtxAction::InspectAvatar(id)) if id == avatar));
        // the two derender entries, always enabled (Object.Derender)
        for (key, permanent) in [("ui-context-derender-temporarily", false), ("ui-context-derender-blacklist", true)] {
            let e = find(world_menu(&on_avatar, &wearing, Vec3::ZERO), key, permanent);
            assert!(e.enabled);
            assert!(matches!(e.kind, Kind::Action(CtxAction::Derender { id, permanent: p }) if id == avatar && p == permanent));
            // on a worn object: that object, not its wearer
            let on_worn = Target::Avatar {
                id: avatar,
                own: false,
                attachment: Some(worn),
            };
            let e = find(world_menu(&on_worn, &wearing, Vec3::ZERO), key, permanent);
            assert!(matches!(e.kind, Kind::Action(CtxAction::Derender { id, .. }) if id == worn));
        }
        // our own worn object: greyed until its prim is known
        let key = ObjKey { region: 1, local_id: 3 };
        let ours = find(
            world_menu(&Target::Attachment { key }, &Facts::default(), Vec3::ZERO),
            "ui-context-derender-temporarily",
            false,
        );
        assert!(!ours.enabled);
    }

    #[test]
    fn worn_shape_edit_is_available_from_our_avatar_and_attachment_menus() {
        let item = Uuid::from_u128(42);
        let facts = Facts {
            worn_shape: Some(item),
            ..Default::default()
        };
        for target in [
            Target::Avatar {
                id: facts.me,
                own: true,
                attachment: None,
            },
            Target::Attachment {
                key: ObjKey { region: 1, local_id: 2 },
            },
        ] {
            let find = |facts: &Facts| {
                world_menu(&target, facts, Vec3::ZERO)
                    .into_iter()
                    .find_map(|entry| {
                        let Kind::Submenu(entries) = entry.kind else { return None };
                        entries
                            .into_iter()
                            .find(|entry| entry.label == aurora_i18n::tr!("ui-context-edit-shape"))
                    })
                    .expect("shape action")
            };
            let entry = find(&facts);
            assert!(entry.enabled);
            assert!(matches!(entry.kind, Kind::Action(CtxAction::EditWorn(id)) if id == item));
            assert!(!find(&Facts::default()).enabled);
        }
    }

    #[test]
    fn our_attachment_has_firestorms_entries_in_order() {
        // menu_attachment_self.xml, without the entries it hides
        let key = ObjKey { region: 1, local_id: 42 };
        let f = Facts::default();
        let got = labels(&world_menu(&Target::Attachment { key }, &f, Vec3::ZERO));
        let expected = tr(&[
            "ui-context-touch",
            "ui-context-show-in-inventory",
            "common-edit",
            "ui-context-edit-pbr-material",
            "common-detach",
            "common-save-as>",
            "-",
            "status-sit",
            "common-stand-up",
            "common-appearance>",
            "common-remove>",
            "-",
            "ui-context-sit-stand",
            "extra-fly-land-8",
            "ui-context-stop-animations",
            "-",
            "ui-context-community>",
            "ui-context-object-profile",
            "ui-context-inspect-2",
            "ui-context-script-information",
            "ui-context-derender-temporarily",
            "ui-context-derender-blacklist",
            "ui-context-show-textures",
            "ui-context-refresh-textures",
            "-",
            "ui-context-drop",
            "-",
            "ui-context-block-particle-owner",
        ]);
        assert_eq!(got, expected);
        // sitting: "Fly / Land" is greyed (enable_fly_land)
        let fly = |f: &Facts| {
            world_menu(&Target::Attachment { key }, f, Vec3::ZERO)
                .into_iter()
                .find(|e| e.label == aurora_i18n::tr!("extra-fly-land-8"))
                .is_some_and(|e| e.enabled)
        };
        assert!(fly(&f));
        assert!(!fly(&Facts {
            seated: true,
            ..Facts::default()
        }));
    }

    const ID: Uuid = Uuid::from_u128(9);

    fn object_target() -> Target {
        Target::Object {
            key: ObjKey { region: 1, local_id: 42 },
            full_id: ID,
            offset: Vec3::ZERO,
        }
    }

    /// Every row of a menu with its submenus, depth first.
    fn rows(entries: Vec<Entry>) -> Vec<Entry> {
        let mut out = Vec::new();
        for e in entries {
            if let Kind::Submenu(children) = &e.kind {
                out.extend(rows(children.clone()));
            }
            out.push(e);
        }
        out
    }

    fn row(entries: Vec<Entry>, key: &'static str) -> Entry {
        let label = aurora_i18n::text(key);
        rows(entries)
            .into_iter()
            .find(|e| e.label == label)
            .unwrap_or_else(|| panic!("{key} missing"))
    }

    #[test]
    fn the_object_menu_has_no_unavailable_row_left_but_the_big_windows() {
        let f = Facts::default();
        let left: Vec<String> = rows(world_menu(&object_target(), &f, Vec3::ZERO))
            .into_iter()
            .filter(|e| matches!(e.kind, Kind::Unavailable))
            .map(|e| e.label)
            .collect();
        // the PBR material editor, the object profile, the pathfinding
        // windows and the exporters are windows of their own; script info
        // waits for its own task
        let expected = tr(&[
            "ui-context-edit-pbr-material",
            "common-profile",
            "ui-context-script-information",
            "ui-context-show-in-region-objects",
            "ui-context-show-in-characters",
            "common-save-as",
        ]);
        let mut left_sorted = left.clone();
        left_sorted.sort();
        let mut expected_sorted = expected;
        expected_sorted.sort();
        assert_eq!(left_sorted, expected_sorted, "{left:?}");
    }

    #[test]
    fn return_scripts_and_put_on_follow_the_object_facts() {
        let mut f = Facts::default();
        let menu = |f: &Facts| world_menu(&object_target(), f, Vec3::ZERO);
        // Return: any object that is not worn
        let ret = row(menu(&f), "ui-context-return");
        assert!(ret.enabled && matches!(ret.kind, Kind::Action(CtxAction::Return(id)) if id == ID));
        // the script actions need a prim we may modify
        assert!(!row(menu(&f), "ui-context-reset-scripts").enabled);
        f.object.scripts = true;
        for (key, kind) in [
            ("ui-context-reset-scripts", QueueKind::Reset),
            ("ui-context-run-scripts", QueueKind::Run),
            ("ui-context-stop-scripts", QueueKind::Stop),
            ("ui-context-remove-scripts", QueueKind::Delete),
        ] {
            let r = row(menu(&f), key);
            assert!(r.enabled);
            assert!(matches!(r.kind, Kind::Action(CtxAction::Scripts { id, action }) if id == ID && action == kind));
        }
        // Put on: Wear replaces on its own point, Add and the points add
        f.attach_points = vec![
            crate::ui::context::AttachChoice {
                id: 6,
                label: "Right Hand".into(),
                group: 1,
                pie_slice: 3,
            },
            crate::ui::context::AttachChoice {
                id: 33,
                label: "Top".into(),
                group: 9,
                pie_slice: 0,
            },
        ];
        assert!(!row(menu(&f), "common-wear").enabled);
        f.object.wear = true;
        assert!(matches!(
            row(menu(&f), "common-wear").kind,
            Kind::Action(CtxAction::Wear { point: 0, add: false, .. })
        ));
        assert!(matches!(
            row(menu(&f), "common-add").kind,
            Kind::Action(CtxAction::Wear { point: 0, add: true, .. })
        ));
        let entries = rows(menu(&f));
        let hand = entries.iter().find(|e| e.label == "Right Hand").expect("body point");
        assert!(hand.enabled && matches!(hand.kind, Kind::Action(CtxAction::Wear { point: 6, add: true, .. })));
        let top = entries.iter().find(|e| e.label == "Top").expect("HUD point");
        assert!(matches!(top.kind, Kind::Action(CtxAction::Wear { point: 33, add: true, .. })));
        // the body point sits in its body part submenu
        let Kind::Submenu(arm) = row(menu(&f), "ui-context-body-right-arm").kind else {
            panic!("not a submenu");
        };
        assert_eq!(arm.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Right Hand"]);
    }

    #[test]
    fn a_kept_selection_links_and_takes_multiple() {
        let mut f = Facts::default();
        f.object.take = true;
        f.object.take_copy = true;
        let menu = |f: &Facts| world_menu(&object_target(), f, Vec3::ZERO);
        // one object: Take / Take copy, nothing to link
        assert!(!row(menu(&f), "ui-context-link").enabled);
        assert!(matches!(row(menu(&f), "ui-context-take").kind, Kind::Action(CtxAction::Take(_))));
        f.object.roots = 3;
        f.object.link = true;
        let link = row(menu(&f), "ui-context-link");
        assert!(link.enabled && matches!(link.kind, Kind::Action(CtxAction::Link(id)) if id == ID));
        let labels = labels(&menu(&f));
        assert!(labels.contains(&format!("{}>", aurora_i18n::text("ui-context-take-multiple"))));
        assert!(!labels.contains(&aurora_i18n::text("ui-context-take").to_string()));
        assert!(matches!(
            row(menu(&f), "ui-context-take-separate").kind,
            Kind::Action(CtxAction::TakeSeparate { copy: false, .. })
        ));
        assert!(matches!(
            row(menu(&f), "ui-context-take-copy-separate").kind,
            Kind::Action(CtxAction::TakeSeparate { copy: true, .. })
        ));
    }

    #[test]
    fn touch_and_sit_say_the_objects_own_texts() {
        let mut f = Facts::default();
        f.object.touch = true;
        f.object.touch_name = "Ring the bell".into();
        f.object.sit_name = "Lie down".into();
        let entries = world_menu(&object_target(), &f, Vec3::ZERO);
        assert!(
            entries
                .iter()
                .any(|e| e.label == "Ring the bell" && matches!(e.kind, Kind::Action(CtxAction::Touch(42))))
        );
        assert!(
            entries
                .iter()
                .any(|e| e.label == "Lie down" && matches!(e.kind, Kind::Action(CtxAction::Sit { .. })))
        );
    }

    #[test]
    fn the_particle_owner_and_the_animesh_skeleton_rows() {
        let owner = Uuid::from_u128(77);
        let mut f = Facts::default();
        let menu = |f: &Facts| world_menu(&object_target(), f, Vec3::ZERO);
        assert!(!row(menu(&f), "ui-context-ignore-particle-owner").enabled);
        // no reset skeleton on a prim that drives no skeleton
        assert!(!labels(&menu(&f)).contains(&aurora_i18n::text("ui-context-reset-skeleton").to_string()));
        f.particle_owner = Some(owner);
        f.object.animesh = Some(ID);
        let block = row(menu(&f), "ui-context-ignore-particle-owner");
        assert!(block.enabled && matches!(block.kind, Kind::Action(CtxAction::BlockParticleOwner(o)) if o == owner));
        assert!(matches!(
            row(menu(&f), "ui-context-reset-skeleton").kind,
            Kind::Action(CtxAction::ResetSkeleton { avatar, animations: false }) if avatar == ID
        ));
        // the land menu blocks the same owner
        let land = row(world_menu(&Target::Ground, &f, Vec3::ZERO), "ui-context-ignore-particle-owner");
        assert!(land.enabled);
        // and our attachments' menu
        let key = ObjKey { region: 1, local_id: 42 };
        let att = row(
            world_menu(&Target::Attachment { key }, &f, Vec3::ZERO),
            "ui-context-block-particle-owner",
        );
        assert!(att.enabled);
    }

    #[test]
    fn our_attachment_drops_and_toggles_its_favorite() {
        let key = ObjKey { region: 1, local_id: 42 };
        let item = Uuid::from_u128(5);
        let mut f = Facts::default();
        let menu = |f: &Facts| world_menu(&Target::Attachment { key }, f, Vec3::ZERO);
        assert!(!row(menu(&f), "ui-context-drop").enabled);
        f.object.drop = true;
        let drop = row(menu(&f), "ui-context-drop");
        assert!(drop.enabled && matches!(drop.kind, Kind::Action(CtxAction::Drop(k)) if k == key));
        // the favorite row, once the item is known: the one that applies
        f.object.attachment_item = Some(item);
        f.object.favorite = Some(false);
        let add = row(menu(&f), "common-add-to-favorites");
        assert!(matches!(add.kind, Kind::Action(CtxAction::AttachmentFavorite(i)) if i == item));
        f.object.favorite = Some(true);
        let entries = menu(&f);
        assert!(!labels(&entries).contains(&aurora_i18n::text("common-add-to-favorites").to_string()));
        row(entries, "common-remove-from-favorites");
    }

    #[test]
    fn a_residents_menus_invite_add_to_contact_sets_and_block_particles() {
        let id = Uuid::from_u128(7);
        let find = |entries: Vec<Entry>, key: &'static str| {
            entries
                .into_iter()
                .find(|e| e.label == aurora_i18n::text(key))
                .unwrap_or_else(|| panic!("{key} missing"))
        };
        for attachment in [None, Some(Uuid::from_u128(8))] {
            let target = Target::Avatar {
                id,
                own: false,
                attachment,
            };
            let f = Facts {
                contact_sets: vec!["Family".into(), "Work".into()],
                ..Facts::default()
            };
            // "Invite to group": enabled, the group picker for them
            let invite = find(world_menu(&target, &f, Vec3::ZERO), "ui-context-invite-to-group");
            assert!(invite.enabled);
            assert!(matches!(invite.kind, Kind::Action(CtxAction::InviteToGroup(i)) if i == id));
            // "Give my calling card": enabled, OfferCallingCard to them
            let card = find(world_menu(&target, &f, Vec3::ZERO), "ui-context-give-my-calling-card");
            assert!(card.enabled);
            assert!(matches!(card.kind, Kind::Action(CtxAction::GiveCallingCard(i)) if i == id));
            // "Add to contact set": each set, then a new one
            let sets = find(world_menu(&target, &f, Vec3::ZERO), "ui-context-add-to-contact-set");
            let Kind::Submenu(rows) = sets.kind else {
                panic!("not a submenu");
            };
            let labels: Vec<&str> = rows.iter().map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["Family", "Work", "", aurora_i18n::tr!("ui-context-new-contact-set")]);
            assert!(matches!(
                &rows[1].kind,
                Kind::Action(CtxAction::ContactSet(crate::ui::contacts::SetEdit::Add { set, ids })) if set == "Work" && ids == &[id]
            ));
            assert!(matches!(&rows[3].kind, Kind::Action(CtxAction::NewContactSet(ids)) if ids == &[id]));
            // no set yet: a greyed line, and still "New contact set…"
            let none = find(world_menu(&target, &Facts::default(), Vec3::ZERO), "ui-context-add-to-contact-set");
            let Kind::Submenu(rows) = none.kind else {
                panic!("not a submenu");
            };
            assert!(!rows[0].enabled);
            assert!(matches!(
                rows.last().map(|e| &e.kind),
                Some(Kind::Action(CtxAction::NewContactSet(_)))
            ));
        }
        // "Block particles" of the avatar's menu: checked while their
        // particles are blocked
        let target = Target::Avatar {
            id,
            own: false,
            attachment: None,
        };
        for blocked in [false, true] {
            let f = Facts {
                particles_blocked: blocked,
                ..Facts::default()
            };
            let row = find(world_menu(&target, &f, Vec3::ZERO), "common-block-particles");
            assert!(row.enabled);
            assert_eq!(row.checked, Some(blocked));
            assert!(matches!(row.kind, Kind::Action(CtxAction::ToggleParticleMute(i)) if i == id));
        }
    }
}

#[cfg(test)]
mod bridge_query_tests {
    use super::*;
    #[test]
    fn object_submenu_dispatches_script_information_for_the_selected_object() {
        let object = Uuid::from_u128(101);
        let target = Target::Object {
            key: ObjKey { region: 1, local_id: 10 },
            full_id: object,
            offset: Vec3::ZERO,
        };
        let entries = world_menu(&target, &Facts::default(), Vec3::ZERO);
        let entry = entries
            .iter()
            .find(|e| e.label == aurora_i18n::tr!("common-object"))
            .expect("object submenu");
        let Kind::Submenu(children) = &entry.kind else { panic!("submenu") };
        let info = children
            .iter()
            .find(|e| e.label == aurora_i18n::tr!("ui-context-script-information"))
            .expect("script info");
        assert!(info.enabled);
        assert!(matches!(&info.kind, Kind::Action(CtxAction::ScriptInfo(id)) if *id == object));
    }
}
