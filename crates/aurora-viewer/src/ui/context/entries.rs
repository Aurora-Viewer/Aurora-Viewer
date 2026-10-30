//! Shared world menu entries and permissions, following Legacy Viewer menu_*.xml.

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

impl Entry {
    /// Checkable settings are applied live; keep their menu and submenu open
    /// for another change (human preference over the Legacy default).
    pub(super) fn keeps_open(&self) -> bool {
        self.checked.is_some() || matches!(self.kind, Kind::Action(CtxAction::CustomizeMenu(_)))
    }
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
    if f.particle_only {
        particles_submenu(&mut entries, f);
        return entries.0;
    }
    if f.object.local_preview {
        if let Target::Object { key, full_id, .. } = target {
            preview_menu(&mut entries, f, *key, *full_id, point, false);
            return entries.0;
        }
        if let Target::Attachment { key } = target {
            preview_menu(&mut entries, f, *key, f.object.id, point, true);
            return entries.0;
        }
    }
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
    if matches!(target, Target::Avatar { own: true, .. }) {
        particles_submenu(&mut entries, f);
    }
    entries.0
}

/// Local linksets keep the normal Edit / Edit linked / Inspect interaction.
/// Inventory, scripts and simulator operations have no local counterpart.
fn preview_menu(ui: &mut Builder, f: &Facts, key: ObjKey, id: Uuid, point: Vec3, worn: bool) {
    ui.item("pencil-filled", aurora_i18n::tr!("common-edit"), CtxAction::Edit(id));
    ui.check(
        "cube-3d-sphere",
        aurora_i18n::tr!("ui-context-edit-linked-parts"),
        f.edit_linked,
        true,
        CtxAction::EditLinkedParts(!f.edit_linked),
    );
    ui.item("zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2"), CtxAction::Inspect(id));
    if f.object.local_hud_scale.is_none() {
        ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point, None));
    }
    if worn {
        ui.item("paperclip", aurora_i18n::tr!("common-detach"), CtxAction::Detach(key));
        ui.item("package", aurora_i18n::tr!("blender-object-drop"), CtxAction::Drop(key));
    } else {
        put_on_submenu(ui, f, id);
        ui.item("trash-filled", aurora_i18n::tr!("common-delete"), CtxAction::Delete(id));
    }
}

#[cfg(test)]
pub(super) fn draw_list(ui: &mut egui::Ui, p: &crate::theme::Palette, entries: &[Entry]) {
    use super::menu;
    for entry in entries {
        match &entry.kind {
            Kind::Action(action) => {
                let clicked = match entry.checked {
                    Some(checked) if entry.enabled => {
                        let mut on = checked;
                        menu::toggle(ui, p, entry.icon, &entry.label, &mut on)
                    }
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
    ui.item_if(
        "paint-filled",
        aurora_i18n::tr!("ui-context-edit-pbr-material"),
        f.object.edit,
        CtxAction::EditPbr(f.object.id),
    );
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
        ui.item(
            "info-circle-filled",
            aurora_i18n::tr!("common-profile"),
            CtxAction::ObjectProfile(id),
        );
        ui.item("zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2"), CtxAction::Inspect(id));
        ui.item("code", aurora_i18n::tr!("ui-context-script-information"), CtxAction::ScriptInfo(id));
        ui.item("box", aurora_i18n::tr!("ui-context-object-information"), CtxAction::ObjectInfo(id));
        ui.separator();
        // two objects or more: a right click on a selected one keeps them;
        // one alone has nothing to link to
        ui.item_if(
            "link",
            aurora_i18n::tr!("ui-context-link"),
            o.roots > 1 && o.link,
            CtxAction::Link(id),
        );
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
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point, None));
    ui.item(
        "list-details-filled",
        aurora_i18n::tr!("ui-context-show-in-region-objects"),
        CtxAction::Pathfinding { id, characters: false },
    );
    ui.item(
        "walk",
        aurora_i18n::tr!("ui-context-show-in-characters"),
        CtxAction::Pathfinding { id, characters: true },
    );
    ui.separator();
    put_on_submenu(ui, f, id);
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        // Object.ReportAbuse: showFromObject on the object
        ui.item(
            "flag-filled",
            aurora_i18n::tr!("common-report"),
            CtxAction::ReportAbuse(ReportTarget::Object(id)),
        );
        ui.check(
            "ban",
            aurora_i18n::tr!("common-ignore"),
            o.blocked,
            true,
            CtxAction::ToggleBlockObject { id, name: o.name.clone() },
        );
        ui.separator();
        derender_items(ui, id, aurora_i18n::tr!("ui-context-derender"), true);
        particles_submenu(ui, f);
    });
    ui.separator();
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshTextures(id),
    );
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

/// Contextual particle controls: direct entries for the clicked emitter,
/// named child groups for every other emitter intersected by the same click.
fn particles_submenu(ui: &mut Builder, f: &Facts) {
    if f.particle_sources.is_empty() {
        return;
    }
    ui.submenu("sparkles-filled", aurora_i18n::tr!("scene-particles-menu"), true, |ui| {
        for source in f.particle_sources.iter().filter(|s| s.clicked) {
            particle_filter_items(ui, source);
        }
        if f.particle_sources.iter().any(|s| s.clicked) && f.particle_sources.iter().any(|s| !s.clicked) {
            ui.separator();
        }
        for source in f.particle_sources.iter().filter(|s| !s.clicked) {
            ui.submenu("box", &source.name, true, |ui| particle_filter_items(ui, source));
        }
    });
}

fn particle_filter_items(ui: &mut Builder, source: &super::ParticleMenuSource) {
    for (id, owner, checked, label) in [
        (
            source.object,
            false,
            source.object_blocked,
            aurora_i18n::tr!("scene-particles-block-object"),
        ),
        (
            source.owner,
            true,
            source.owner_blocked,
            aurora_i18n::tr!("scene-particles-block-owner"),
        ),
    ] {
        ui.check(
            "sparkles-filled",
            label,
            checked,
            !id.is_nil(),
            CtxAction::SetParticleBlock {
                id,
                owner,
                blocked: !checked,
            },
        );
    }
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
        ui.check(
            "lock-filled",
            aurora_i18n::tr!("hud-move-locked"),
            f.object.hud_move_locked,
            true,
            CtxAction::SetHudMoveLock(key, !f.object.hud_move_locked),
        );
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
        let (icon, label) = if favorite {
            ("star-filled", aurora_i18n::tr!("common-remove-from-favorites"))
        } else {
            ("star", aurora_i18n::tr!("common-add-to-favorites"))
        };
        ui.item(icon, label, CtxAction::AttachmentFavorite(item));
    }
    ui.item_if(
        "pencil-filled",
        aurora_i18n::tr!("common-edit"),
        f.object.edit,
        CtxAction::EditAttachment(key),
    );
    ui.item_if(
        "paint-filled",
        aurora_i18n::tr!("ui-context-edit-pbr-material"),
        f.object.edit,
        CtxAction::EditPbr(f.object.id),
    );
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
    take_off_submenu(ui, f);
    ui.separator();
    // Self.ToggleSitStand
    ui.item("armchair", aurora_i18n::tr!("ui-context-sit-stand"), CtxAction::SitStand);
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
    ui.item_if(
        "info-circle-filled",
        aurora_i18n::tr!("ui-context-object-profile"),
        !f.object.id.is_nil(),
        CtxAction::ObjectProfile(f.object.id),
    );
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
    ui.item_if(
        "box",
        aurora_i18n::tr!("ui-context-object-information"),
        !f.object.id.is_nil(),
        CtxAction::ObjectInfo(f.object.id),
    );
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        derender_items(
            ui,
            f.object.id,
            aurora_i18n::tr!("ui-context-derender-temporarily"),
            !f.object.id.is_nil(),
        );
        particles_submenu(ui, f);
    });
    ui.item(
        "photo-filled",
        aurora_i18n::tr!("ui-context-show-textures"),
        CtxAction::ShowAvatarTextures(f.me),
    );
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
}

fn avatar_navigation(ui: &mut Builder, f: &Facts, id: Uuid) {
    ui.item_if(
        "walk",
        aurora_i18n::tr!("ui-context-go-to-avatar"),
        f.avatar_travel,
        CtxAction::GoToAvatar(id),
    );
    let (label, action) = if f.following {
        (aurora_i18n::tr!("ui-context-stop-following"), CtxAction::StopFollowing)
    } else {
        (aurora_i18n::tr!("common-follow"), CtxAction::FollowAvatar(id))
    };
    ui.item_if("current-location-filled", label, f.avatar_travel || f.following, action);
}

fn sound_block_items(ui: &mut Builder, f: &Facts, owner: Uuid) {
    for (flag, label) in [
        (1, aurora_i18n::tr!("ui-context-block-worn-sounds")),
        (2, aurora_i18n::tr!("ui-context-block-rezzed-sounds")),
        (4, aurora_i18n::tr!("ui-context-block-gesture-sounds")),
    ] {
        let on = f.sound_blocks & flag != 0;
        ui.check(
            "volume-off",
            label,
            on,
            true,
            CtxAction::SetOwnerSoundBlock { owner, flag, blocked: !on },
        );
    }
}

/// Legacy menu_avatar_other.xml / menu_attachment_other.xml actions, grouped
/// deliberately for Aurora (human request and #1135). Keep frequent social
/// actions direct and technical/safety actions out of their click path.
fn avatar_social_items(ui: &mut Builder, f: &Facts, id: Uuid) {
    ui.item("user-filled", aurora_i18n::tr!("common-view-profile"), CtxAction::Profile(id));
    avatar_navigation(ui, f, id);
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
}

fn avatar_menu(ui: &mut Builder, f: &Facts, id: Uuid, point: Vec3) {
    avatar_social_items(ui, f, id);
    avatar_groups(ui, f, id, None, point);
}

fn attachment_other_menu(ui: &mut Builder, f: &Facts, id: Uuid, attachment: Uuid, point: Vec3) {
    avatar_social_items(ui, f, id);
    avatar_groups(ui, f, id, Some(attachment), point);
}

fn avatar_groups(ui: &mut Builder, f: &Facts, id: Uuid, attachment: Option<Uuid>, point: Vec3) {
    ui.separator();
    ui.item(
        "eye-filled",
        aurora_i18n::tr!("common-look-toward-avatar"),
        CtxAction::LookAtAvatar(id),
    );
    ui.item(
        "zoom-in-filled",
        aurora_i18n::tr!("common-zoom-2"),
        CtxAction::Zoom(point, Some(id)),
    );
    ui.item("coin-filled", aurora_i18n::tr!("common-pay"), CtxAction::PayResident(id));
    ui.separator();
    if let Some(object) = attachment {
        ui.submenu("box", aurora_i18n::tr!("ui-context-clicked-object"), true, |ui| {
            ui.item(
                "info-circle-filled",
                aurora_i18n::tr!("ui-context-object-profile"),
                CtxAction::ObjectProfile(object),
            );
            ui.item(
                "zoom-scan-filled",
                aurora_i18n::tr!("ui-context-inspect-2"),
                CtxAction::Inspect(object),
            );
            ui.item("code", aurora_i18n::tr!("ui-context-script-info"), CtxAction::ScriptInfo(object));
            ui.item(
                "box",
                aurora_i18n::tr!("ui-context-object-information"),
                CtxAction::ObjectInfo(object),
            );
            ui.separator();
            ui.item(
                "refresh",
                aurora_i18n::tr!("ui-context-refresh-textures"),
                CtxAction::RefreshTextures(object),
            );
            save_as_submenu(ui);
        });
    }
    avatar_moderation(ui, f, id);
    let display = f
        .complexity
        .map(|(c, silhouette)| {
            aurora_i18n::tr!(
                "ui-context-complexity",
                value0 = crate::ui::hud::group_digits(c).to_string(),
                value1 = (if silhouette { " (silhouette)" } else { "" }).to_string()
            )
            .to_string()
        })
        .unwrap_or_else(|| aurora_i18n::tr!("ui-context-display-options").into());
    ui.submenu("gauge-filled", &display, true, |ui| {
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
        ui.separator();
        ui.item(
            "photo-filled",
            aurora_i18n::tr!("ui-context-show-textures"),
            CtxAction::ShowAvatarTextures(id),
        );
        ui.item(
            "refresh",
            aurora_i18n::tr!("ui-context-refresh-textures"),
            CtxAction::RefreshAvatarTextures(id),
        );
    });
    ui.submenu(
        "info-circle-filled",
        aurora_i18n::tr!("ui-context-information-options"),
        true,
        |ui| {
            ui.item_if(
                "zoom-scan-filled",
                aurora_i18n::tr!("ui-context-inspect-2"),
                f.worn,
                CtxAction::InspectAvatar(id),
            );
            ui.item("code", aurora_i18n::tr!("ui-context-script-information"), CtxAction::ScriptInfo(id));
        },
    );
    ui.submenu("restore", aurora_i18n::tr!("ui-context-reset-options"), true, |ui| {
        reset_skeleton_items(ui, id);
        ui.item(
            "restore",
            aurora_i18n::tr!("ui-context-reset-mesh-lod-2"),
            CtxAction::ResetMeshLod(id),
        );
    });
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        ui.item(
            "flag-filled",
            aurora_i18n::tr!("common-report"),
            CtxAction::ReportAbuse(ReportTarget::Object(id)),
        );
        ui.check(
            "ban",
            aurora_i18n::tr!("common-ignore"),
            f.blocked,
            true,
            CtxAction::ToggleBlock(id),
        );
        // Local mute of their nearby/call voice only (flagVoiceChat), same as
        // People › Block voice and the menu volume row — not land moderation.
        ui.check(
            "microphone-off",
            aurora_i18n::tr!("ui-people-block-voice"),
            f.voice_blocked,
            f.can_block_voice,
            CtxAction::ToggleVoiceMute(id),
        );
        ui.separator();
        if let Some(object) = attachment {
            // Derender on a clicked attachment has always targeted that
            // object, not its wearer. Make that distinction explicit.
            ui.submenu("box", aurora_i18n::tr!("ui-context-clicked-object"), true, |ui| {
                derender_items(ui, object, aurora_i18n::tr!("ui-context-derender-temporarily"), true);
            });
        } else {
            derender_items(ui, id, aurora_i18n::tr!("ui-context-derender-temporarily"), true);
        }
        ui.check(
            "sparkles-filled",
            aurora_i18n::tr!("common-block-particles"),
            f.particles_blocked,
            true,
            CtxAction::ToggleOwnerParticleFilter(id),
        );
        sound_block_items(ui, f, id);
        particles_submenu(ui, f);
    });
}

/// Visible only with the corresponding land/nearby-voice moderation rights.
fn avatar_moderation(ui: &mut Builder, f: &Facts, id: Uuid) {
    let voice = f.voice_volume.and_then(|v| v.moderated);
    if !f.moderation.allowed && voice.is_none() {
        return;
    }
    ui.submenu("shield-filled", aurora_i18n::tr!("ui-context-moderation-options"), true, |ui| {
        moderation_items(ui, f, id);
        if let Some(muted) = voice {
            ui.check(
                "microphone-off",
                aurora_i18n::tr!("ui-context-voice-mute-for-everyone"),
                muted,
                true,
                CtxAction::ModerateVoice(Some(id), !muted),
            );
            ui.item(
                "microphone-off",
                aurora_i18n::tr!("ui-context-voice-mute-everyone"),
                CtxAction::ModerateVoice(None, true),
            );
            ui.item(
                "microphone-filled",
                aurora_i18n::tr!("ui-context-voice-unmute-everyone"),
                CtxAction::ModerateVoice(None, false),
            );
        }
    });
}

/// Avatar.Freeze / Avatar.Eject, visible only with rights on the avatar's land.
fn moderation_items(ui: &mut Builder, f: &Facts, id: Uuid) {
    if f.moderation.allowed {
        ui.item("snowflake", aurora_i18n::tr!("ui-context-freeze"), CtxAction::Freeze(id));
        ui.item("player-eject-filled", aurora_i18n::tr!("ui-context-eject"), CtxAction::Eject(id));
    }
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
    take_off_submenu(ui, f);
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
    ui.item("code", aurora_i18n::tr!("ui-context-script-information"), CtxAction::ScriptInfo(id));
    ui.item(
        "photo-filled",
        aurora_i18n::tr!("ui-context-show-textures"),
        CtxAction::ShowAvatarTextures(id),
    );
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

fn take_off_submenu(ui: &mut Builder, f: &Facts) {
    ui.submenu("shirt-filled", aurora_i18n::tr!("common-remove"), true, |ui| {
        ui.submenu(
            "shirt-filled",
            aurora_i18n::tr!("common-clothing"),
            !f.removable_clothing.is_empty(),
            |ui| {
                for kind in 4u8..=16 {
                    if let Some(label) = crate::world::appearance::wearables().get(usize::from(kind)) {
                        ui.item_if(
                            "shirt-filled",
                            label,
                            f.removable_clothing.contains(&kind),
                            CtxAction::TakeOffClothing(kind),
                        );
                    }
                }
            },
        );
        for (hud, icon, label) in [
            (true, "device-desktop-filled", aurora_i18n::tr!("scene-menu-huds")),
            (false, "paperclip", aurora_i18n::tr!("common-detach")),
        ] {
            let points: Vec<_> = f.attach_points.iter().filter(|p| (p.group == 9) == hud).collect();
            ui.submenu(icon, label, points.iter().any(|p| f.removable_points.contains(&p.id)), |ui| {
                for point in &points {
                    ui.item_if(
                        icon,
                        &point.label,
                        f.removable_points.contains(&point.id),
                        CtxAction::DetachAt(point.id),
                    );
                }
            });
        }
        ui.item_if(
            "circle-x-filled",
            aurora_i18n::tr!("ui-context-detach-all"),
            !f.removable_points.is_empty(),
            CtxAction::DetachAll,
        );
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
        ui.item(
            "restore",
            aurora_i18n::tr!("ui-context-reset-mesh-lod"),
            CtxAction::ResetMeshLod(f.me),
        );
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
    // LLLandSit::handleEvent / near_sit_down_point.
    ui.item_if(
        "armchair",
        aurora_i18n::tr!("ui-context-sit-here"),
        !f.seated && !f.flying,
        CtxAction::SitAt(point),
    );
    if f.seated {
        ui.item("man-filled", aurora_i18n::tr!("common-stand-up"), CtxAction::StandUp);
    }
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point, None));
    ui.separator();
    // Firestorm greys it out when the parcel is not for sale to us; the
    // menu does not know the parcel before it is selected, so the window
    // says why instead (and a notice for a pass)
    ui.item(
        "shopping-cart-filled",
        aurora_i18n::tr!("ui-context-buy-this-land"),
        CtxAction::BuyLand(point),
    );
    ui.item(
        "ticket-filled",
        aurora_i18n::tr!("ui-context-buy-access"),
        CtxAction::BuyPass(point),
    );
    ui.separator();
    ui.item("box", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item("mountain-filled", aurora_i18n::tr!("ui-context-edit-land"), CtxAction::EditTerrain);
    ui.check(
        "border-all",
        aurora_i18n::tr!("common-show-boundaries"),
        f.property_lines,
        true,
        CtxAction::SetPropertyLines(!f.property_lines),
    );
    ui.separator();
    particles_submenu(ui, f);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn particle_source(object: u128, owner: u128, name: &str, clicked: bool) -> super::super::ParticleMenuSource {
        super::super::ParticleMenuSource {
            object: Uuid::from_u128(object),
            owner: Uuid::from_u128(owner),
            name: name.into(),
            clicked,
            object_blocked: false,
            owner_blocked: false,
        }
    }

    #[test]
    fn particles_show_primary_controls_and_distinct_named_intersected_emitters() {
        for target in [
            object_target(),
            Target::Ground,
            Target::Attachment {
                key: ObjKey { region: 1, local_id: 42 },
            },
            Target::Avatar {
                id: Uuid::from_u128(4),
                own: false,
                attachment: None,
            },
        ] {
            let mut facts = Facts {
                particle_sources: vec![
                    particle_source(1, 10, "Clicked emitter", true),
                    particle_source(2, 20, "Smoke", false),
                    particle_source(3, 30, "Sparkles", false),
                ],
                ..Default::default()
            };
            facts.particle_sources[1].object_blocked = true;
            facts.particle_sources[2].owner_blocked = true;
            let entries = rows(world_menu(&target, &facts, Vec3::ZERO));
            assert!(
                !entries
                    .iter()
                    .any(|e| e.label == aurora_i18n::tr!("ui-context-ignore-particle-owner"))
            );
            let menu = entries
                .iter()
                .find(|e| e.label == aurora_i18n::tr!("scene-particles-menu"))
                .expect("Particles");
            let Kind::Submenu(children) = &menu.kind else { panic!("submenu") };
            assert!(
                matches!(children[0].kind, Kind::Action(CtxAction::SetParticleBlock { id, owner: false, blocked: true }) if id == Uuid::from_u128(1))
            );
            assert!(
                matches!(children[1].kind, Kind::Action(CtxAction::SetParticleBlock { id, owner: true, blocked: true }) if id == Uuid::from_u128(10))
            );
            for (name, object, owner) in [("Smoke", 2, 20), ("Sparkles", 3, 30)] {
                let entry = children.iter().find(|e| e.label == name).expect("named emitter");
                let Kind::Submenu(actions) = &entry.kind else {
                    panic!("source submenu")
                };
                assert_eq!(actions.len(), 2);
                assert!(
                    matches!(actions[0].kind, Kind::Action(CtxAction::SetParticleBlock { id, owner: false, .. }) if id == Uuid::from_u128(object))
                );
                assert!(
                    matches!(actions[1].kind, Kind::Action(CtxAction::SetParticleBlock { id, owner: true, .. }) if id == Uuid::from_u128(owner))
                );
                assert_eq!(actions[0].checked, Some(object == 2));
                assert_eq!(actions[1].checked, Some(object == 3));
            }
        }
        assert!(
            !rows(world_menu(&object_target(), &Facts::default(), Vec3::ZERO))
                .iter()
                .any(|e| e.label == aurora_i18n::tr!("scene-particles-menu"))
        );
    }

    #[test]
    fn particles_over_sky_show_only_the_particle_menu() {
        let facts = Facts {
            particle_sources: vec![particle_source(1, 10, "Smoke", false)],
            particle_only: true,
            ..Default::default()
        };
        let entries = world_menu(&Target::Ground, &facts, Vec3::ZERO);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, aurora_i18n::tr!("scene-particles-menu"));
    }

    #[test]
    fn derender_is_grouped_in_disturbance_and_keeps_the_clicked_target() {
        let avatar = Uuid::from_u128(7);
        let object = Uuid::from_u128(8);
        let key = ObjKey { region: 1, local_id: 42 };
        let mut facts = Facts::default();
        facts.object.id = object;
        for target in [
            Target::Object {
                key,
                full_id: object,
                offset: Vec3::ZERO,
            },
            Target::Attachment { key },
            Target::Avatar {
                id: avatar,
                own: false,
                attachment: None,
            },
            Target::Avatar {
                id: avatar,
                own: false,
                attachment: Some(object),
            },
        ] {
            let menu = world_menu(&target, &facts, Vec3::ZERO);
            assert!(!menu.iter().any(|e| matches!(e.kind, Kind::Action(CtxAction::Derender { .. }))));
            let disturbance = menu
                .into_iter()
                .find(|e| e.label == aurora_i18n::tr!("ui-context-disturbance"))
                .expect("disturbance");
            let Kind::Submenu(children) = disturbance.kind else {
                panic!("submenu")
            };
            let actions = rows(children);
            let expected = if matches!(target, Target::Avatar { attachment: None, .. }) {
                avatar
            } else {
                object
            };
            for permanent in [false, true] {
                assert!(
                    actions.iter().any(
                        |e| matches!(e.kind, Kind::Action(CtxAction::Derender { id, permanent: p }) if id == expected && p == permanent)
                    )
                );
            }
        }
    }

    #[test]
    fn avatar_moderation_group_tracks_land_and_voice_rights_independently() {
        let id = Uuid::from_u128(7);
        let target = Target::Avatar {
            id,
            own: false,
            attachment: None,
        };
        for land in [false, true] {
            for voice in [None, Some(false), Some(true)] {
                let mut facts = Facts::default();
                facts.moderation.allowed = land;
                facts.voice_volume = Some(crate::ui::menu::VoiceVolume {
                    volume: 0.5,
                    muted: false,
                    can_mute: true,
                    moderated: voice,
                });
                let menu = world_menu(&target, &facts, Vec3::ZERO);
                let group = menu
                    .into_iter()
                    .find(|e| e.label == aurora_i18n::tr!("ui-context-moderation-options"));
                assert_eq!(group.is_some(), land || voice.is_some());
                let actions = rows(group.into_iter().collect());
                assert_eq!(
                    actions
                        .iter()
                        .any(|e| matches!(e.kind, Kind::Action(CtxAction::Freeze(who)) if who == id)),
                    land
                );
                assert_eq!(
                    actions
                        .iter()
                        .any(|e| matches!(e.kind, Kind::Action(CtxAction::Eject(who)) if who == id)),
                    land
                );
                let commands: Vec<_> = actions
                    .iter()
                    .filter_map(|e| {
                        if let Kind::Action(CtxAction::ModerateVoice(who, muted)) = e.kind {
                            Some((who, muted))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(
                    commands,
                    voice
                        .map(|muted| vec![(Some(id), !muted), (None, true), (None, false)])
                        .unwrap_or_default()
                );
            }
        }
    }

    #[test]
    fn avatar_navigation_addresses_the_wearer_and_keeps_stop_available() {
        let id = Uuid::from_u128(7);
        for attachment in [None, Some(Uuid::from_u128(8))] {
            let target = Target::Avatar {
                id,
                own: false,
                attachment,
            };
            let mut facts = Facts {
                avatar_travel: true,
                ..Default::default()
            };
            let entries = world_menu(&target, &facts, Vec3::ZERO);
            assert!(
                entries
                    .iter()
                    .any(|e| e.enabled && matches!(e.kind, Kind::Action(CtxAction::GoToAvatar(who)) if who == id))
            );
            assert!(
                entries
                    .iter()
                    .any(|e| e.enabled && matches!(e.kind, Kind::Action(CtxAction::FollowAvatar(who)) if who == id))
            );
            facts.avatar_travel = false;
            facts.following = true;
            let entries = world_menu(&target, &facts, Vec3::ZERO);
            assert!(
                entries
                    .iter()
                    .any(|e| e.enabled && matches!(e.kind, Kind::Action(CtxAction::StopFollowing)))
            );
            assert!(
                entries
                    .iter()
                    .any(|e| !e.enabled && matches!(e.kind, Kind::Action(CtxAction::GoToAvatar(_))))
            );
        }
    }

    #[test]
    fn hud_move_lock_menu_is_checked_reversible_and_available_without_modify_permission() {
        let key = ObjKey { region: 1, local_id: 2 };
        let mut facts = Facts::default();
        facts.object.local_hud_scale = Some(1.0);
        for locked in [false, true] {
            facts.object.hud_move_locked = locked;
            let entries = world_menu(&Target::Attachment { key }, &facts, Vec3::ZERO);
            let entry = entries
                .iter()
                .find(|entry| matches!(entry.kind, Kind::Action(CtxAction::SetHudMoveLock(..))))
                .unwrap();
            assert!(entry.enabled);
            assert_eq!(entry.checked, Some(locked));
            assert!(matches!(entry.kind, Kind::Action(CtxAction::SetHudMoveLock(k, value)) if k == key && value != locked));
        }
        facts.object.local_hud_scale = None;
        assert!(
            !world_menu(&Target::Attachment { key }, &facts, Vec3::ZERO)
                .iter()
                .any(|entry| matches!(entry.kind, Kind::Action(CtxAction::SetHudMoveLock(..))))
        );
    }

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
    fn avatar_and_attachment_menus_group_technical_actions_below_social_actions() {
        for attachment in [None, Some(Uuid::from_u128(8))] {
            let target = Target::Avatar {
                id: Uuid::from_u128(7),
                own: false,
                attachment,
            };
            let f = Facts::default();
            let mut expected = tr(&[
                "common-view-profile",
                "ui-context-go-to-avatar",
                "common-follow",
                "common-become-friends",
                "ui-context-add-to-contact-set>",
                "ui-context-send-im",
                "ui-context-give-my-calling-card",
                "common-voice-call",
                "ui-context-invite-to-group",
                "common-offer-teleport",
                "-",
                "common-look-toward-avatar",
                "common-zoom-2",
                "common-pay",
                "-",
            ]);
            if attachment.is_some() {
                expected.extend(tr(&["ui-context-clicked-object>"]));
            }
            expected.extend(tr(&[
                "ui-context-display-options>",
                "ui-context-information-options>",
                "ui-context-reset-options>",
                "ui-context-disturbance>",
            ]));
            assert_eq!(labels(&world_menu(&target, &f, Vec3::ZERO)), expected);
            let all = rows(world_menu(&target, &f, Vec3::ZERO));
            assert!(
                all.iter()
                    .any(|e| matches!(e.kind, Kind::Action(CtxAction::Zoom(_, Some(id))) if id == Uuid::from_u128(7))),
                "zoom follows the wearer from either avatar menu"
            );
            for mode in [0, 1, 2] {
                assert!(
                    all.iter()
                        .any(|e| matches!(e.kind, Kind::Action(CtxAction::SetRender(_, m)) if m == mode))
                );
            }
        }
    }

    #[test]
    fn inspect_and_derender_act_on_the_avatar_or_on_the_worn_object() {
        let find = |entries: Vec<Entry>, key: &'static str, permanent: bool| {
            rows(entries)
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
    fn our_attachment_keeps_legacy_actions_with_derender_grouped_in_disturbance() {
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
            "ui-context-object-information",
            "ui-context-disturbance>",
            "ui-context-show-textures",
            "ui-context-refresh-textures",
            "-",
            "ui-context-drop",
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
    pub(super) fn rows(entries: Vec<Entry>) -> Vec<Entry> {
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
        // The pathfinding windows and exporters are separate subsystems. Script info
        // is available through the Aurora bridge.
        let expected = tr(&["common-save-as"]);
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
    fn particle_menus_preserve_animesh_skeleton_actions() {
        let mut f = Facts::default();
        f.object.animesh = Some(ID);
        f.particle_sources.push(particle_source(1, 10, "Emitter", true));
        let entries = world_menu(&object_target(), &f, Vec3::ZERO);
        assert!(
            matches!(row(entries, "ui-context-reset-skeleton").kind, Kind::Action(CtxAction::ResetSkeleton { avatar, animations: false }) if avatar == ID)
        );
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
            rows(entries)
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
            assert!(matches!(row.kind, Kind::Action(CtxAction::ToggleOwnerParticleFilter(i)) if i == id));
        }
        // Disturbance › Block voice: mute list voice flag, greyed for Lindens
        for (voice_blocked, can_block_voice) in [(false, true), (true, true), (false, false)] {
            let f = Facts {
                voice_blocked,
                can_block_voice,
                ..Facts::default()
            };
            let row = find(world_menu(&target, &f, Vec3::ZERO), "ui-people-block-voice");
            assert_eq!(row.enabled, can_block_voice);
            assert_eq!(row.checked, Some(voice_blocked));
            assert!(matches!(row.kind, Kind::Action(CtxAction::ToggleVoiceMute(i)) if i == id));
        }
    }
}

#[cfg(test)]
mod bridge_query_tests {
    use super::*;
    #[test]
    fn local_linksets_use_native_edit_linked_inspect_and_attachment_actions() {
        let id = Uuid::from_u128(101);
        let key = ObjKey {
            region: crate::world::objects::PREVIEW_REGION,
            local_id: 1,
        };
        let mut facts = Facts::default();
        facts.object.local_preview = true;
        facts.object.id = id;
        facts.object.wear = true;
        let world = world_menu(
            &Target::Object {
                key,
                full_id: id,
                offset: Vec3::ZERO,
            },
            &facts,
            Vec3::ZERO,
        );
        assert!(
            world
                .iter()
                .any(|e| e.enabled && matches!(e.kind,Kind::Action(CtxAction::Edit(who)) if who==id))
        );
        let toggle = world
            .iter()
            .find(|e| matches!(e.kind, Kind::Action(CtxAction::EditLinkedParts(true))))
            .expect("explicit part editing");
        assert_eq!(toggle.checked, Some(false));
        assert!(
            world
                .iter()
                .any(|e| matches!(e.kind,Kind::Action(CtxAction::Inspect(who)) if who==id))
        );
        assert!(
            world
                .iter()
                .any(|e| matches!(e.kind,Kind::Action(CtxAction::Delete(who)) if who==id))
        );
        assert!(!world.iter().any(|e| matches!(
            e.kind,
            Kind::Action(CtxAction::Sit { .. } | CtxAction::Take(_) | CtxAction::Pay(_) | CtxAction::Return(_))
        )));
        facts.edit_linked = true;
        let worn = world_menu(&Target::Attachment { key }, &facts, Vec3::ZERO);
        assert!(worn.iter().any(|e| matches!(e.kind,Kind::Action(CtxAction::Detach(k)) if k==key)));
        assert!(worn.iter().any(|e| matches!(e.kind,Kind::Action(CtxAction::Drop(k)) if k==key)));
        assert!(
            worn.iter()
                .any(|e| e.checked == Some(true) && matches!(e.kind, Kind::Action(CtxAction::EditLinkedParts(false))))
        );
    }

    #[test]
    fn object_submenu_dispatches_separate_script_and_extended_information_for_the_selected_object() {
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
        let index = children
            .iter()
            .position(|e| e.label == aurora_i18n::tr!("ui-context-script-information"))
            .expect("script info");
        let info = &children[index];
        assert!(info.enabled);
        assert!(matches!(&info.kind, Kind::Action(CtxAction::ScriptInfo(id)) if *id == object));
        let extended = &children[index + 1];
        assert!(extended.enabled);
        assert_eq!(extended.label, aurora_i18n::tr!("ui-context-object-information"));
        assert!(matches!(&extended.kind, Kind::Action(CtxAction::ObjectInfo(id)) if *id == object));
    }

    #[test]
    fn attachment_information_uses_the_worn_object_and_follows_script_information() {
        let object = Uuid::from_u128(102);
        for target in [
            Target::Attachment {
                key: ObjKey { region: 1, local_id: 10 },
            },
            Target::Avatar {
                id: Uuid::from_u128(7),
                own: false,
                attachment: Some(object),
            },
        ] {
            let facts = Facts {
                object: super::super::ObjectFacts {
                    id: object,
                    ..Default::default()
                },
                ..Facts::default()
            };
            let entries = super::tests::rows(world_menu(&target, &facts, Vec3::ZERO));
            let index = entries
                .iter()
                .position(|e| matches!(e.kind, Kind::Action(CtxAction::ScriptInfo(id)) if id == object))
                .expect("script info");
            let extended = &entries[index + 1];
            assert!(extended.enabled);
            assert!(matches!(&extended.kind, Kind::Action(CtxAction::ObjectInfo(id)) if *id == object));
        }
    }
}
