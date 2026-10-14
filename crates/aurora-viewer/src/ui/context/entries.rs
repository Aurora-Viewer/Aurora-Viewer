//! Shared world menu entries and permissions, following Firestorm menu_*.xml.

use super::{CtxAction, Facts, Target};
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
    ui.item_if(
        "click-filled",
        aurora_i18n::tr!("ui-context-touch"),
        o.touch,
        CtxAction::Touch(key.local_id),
    );
    ui.item("pencil-filled", aurora_i18n::tr!("common-edit"), CtxAction::Edit(id));
    ui.todo("paint-filled", aurora_i18n::tr!("ui-context-edit-pbr-material"));
    // Object.Build: the Create tool (LLToolCompCreate), like the land menu
    ui.item("box", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item_if("package", aurora_i18n::tr!("common-open"), o.open, CtxAction::Open(id));
    ui.separator();
    ui.item_if(
        "armchair",
        aurora_i18n::tr!("ui-context-sit-here"),
        o.sit,
        CtxAction::Sit { target: id, offset },
    );
    ui.separator();
    ui.item_if("man-filled", aurora_i18n::tr!("common-stand-up"), o.stand, CtxAction::StandUp);
    ui.separator();
    ui.submenu("box", aurora_i18n::tr!("common-object"), true, |ui| {
        ui.todo("info-circle-filled", aurora_i18n::tr!("common-profile"));
        ui.item("zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2"), CtxAction::Inspect(id));
        ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
        ui.separator();
        // linking needs two selected objects: build tools only
        ui.disabled("link", aurora_i18n::tr!("ui-context-link"));
        ui.item_if("unlink", aurora_i18n::tr!("ui-context-unlink"), o.unlink, CtxAction::Unlink(id));
        ui.check(
            "cube-3d-sphere",
            aurora_i18n::tr!("ui-context-edit-linked-parts"),
            f.edit_linked,
            true,
            CtxAction::EditLinkedParts(!f.edit_linked),
        );
        ui.separator();
        ui.todo("refresh", aurora_i18n::tr!("ui-context-reset-scripts"));
        ui.todo("player-play-filled", aurora_i18n::tr!("ui-context-run-scripts"));
        ui.todo("player-stop-filled", aurora_i18n::tr!("ui-context-stop-scripts"));
        ui.todo("trash-filled", aurora_i18n::tr!("ui-context-remove-scripts"));
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.separator();
    ui.submenu("shirt-filled", aurora_i18n::tr!("ui-context-wear"), true, |ui| {
        ui.todo("shirt-filled", aurora_i18n::tr!("common-wear"));
        ui.todo("circle-plus-filled", aurora_i18n::tr!("common-add"));
        ui.todo("paperclip", aurora_i18n::tr!("common-attach-to"));
        ui.todo("paperclip", aurora_i18n::tr!("common-attach-to-hud"));
    });
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        ui.todo("flag-filled", aurora_i18n::tr!("common-report"));
        let (icon, label) = if o.blocked {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("forbid-filled", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlockObject { id, name: o.name.clone() });
    });
    ui.separator();
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshTextures(id),
    );
    ui.todo("eye-off", aurora_i18n::tr!("ui-context-derender"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("arrow-back-up", aurora_i18n::tr!("ui-context-return"));
    ui.item_if("hand-grab", aurora_i18n::tr!("ui-context-take"), o.take, CtxAction::Take(id));
    ui.item_if(
        "copy-filled",
        aurora_i18n::tr!("ui-context-take-copy"),
        o.take_copy,
        CtxAction::TakeCopy(id),
    );
    ui.item_if("coin-filled", aurora_i18n::tr!("common-pay"), o.pay, CtxAction::Pay(id));
    ui.item_if("shopping-cart-filled", aurora_i18n::tr!("common-buy"), o.buy, CtxAction::Buy(id));
    ui.todo("device-floppy-filled", aurora_i18n::tr!("common-save-as"));
    ui.separator();
    ui.item_if("trash-filled", aurora_i18n::tr!("common-delete"), o.delete, CtxAction::Delete(id));
    ui.separator();
    ui.todo("sparkles-filled", aurora_i18n::tr!("ui-context-ignore-particle-owner"));
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
        aurora_i18n::tr!("ui-context-touch"),
        f.object.touch,
        CtxAction::Touch(key.local_id),
    );
    ui.item_if(
        "briefcase-filled",
        aurora_i18n::tr!("ui-context-show-in-inventory"),
        f.object.attachment_item.is_some(),
        CtxAction::AttachmentInventory(f.object.attachment_item.unwrap_or_default()),
    );
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
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("eye-off", aurora_i18n::tr!("ui-context-derender-temporarily"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    ui.item_if(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        !f.object.id.is_nil(),
        CtxAction::RefreshTextures(f.object.id),
    );
    ui.separator();
    // "Delete" drops the attachment on the ground (Attachment.Drop)
    ui.todo("trash-filled", aurora_i18n::tr!("common-delete"));
    ui.separator();
    ui.todo("sparkles-filled", aurora_i18n::tr!("ui-context-block-particle-owner"));
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
    ui.todo("users-group", aurora_i18n::tr!("ui-context-add-to-contact-set"));
    ui.item("message-filled", aurora_i18n::tr!("ui-context-send-im"), CtxAction::Im(id));
    ui.todo("id-filled", aurora_i18n::tr!("ui-context-give-my-calling-card"));
    ui.todo("phone-filled", aurora_i18n::tr!("common-voice-call"));
    ui.todo("users-plus", aurora_i18n::tr!("ui-context-invite-to-group"));
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
            ("forbid-filled", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlock(id));
        ui.todo("flag-filled", aurora_i18n::tr!("common-report"));
        ui.todo("snowflake", aurora_i18n::tr!("ui-context-freeze"));
        ui.todo("player-eject-filled", aurora_i18n::tr!("ui-context-eject"));
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.todo("coin-filled", aurora_i18n::tr!("common-pay"));
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
    ui.todo("zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2"));
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("eye-off", aurora_i18n::tr!("ui-context-derender-temporarily"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshAvatarTextures(id),
    );
    ui.separator();
    ui.todo("sparkles-filled", aurora_i18n::tr!("common-block-particles"));
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
    ui.todo("users-group", aurora_i18n::tr!("ui-context-add-to-contact-set"));
    ui.item("message-filled", aurora_i18n::tr!("ui-context-send-im"), CtxAction::Im(id));
    ui.todo("id-filled", aurora_i18n::tr!("ui-context-give-my-calling-card"));
    ui.todo("phone-filled", aurora_i18n::tr!("common-voice-call"));
    ui.todo("users-plus", aurora_i18n::tr!("ui-context-invite-to-group"));
    ui.todo("eye-filled", aurora_i18n::tr!("common-look-toward-avatar"));
    ui.separator();
    reset_skeleton_items(ui, id);
    ui.todo("restore", aurora_i18n::tr!("ui-context-reset-mesh-lod-2"));
    ui.separator();
    ui.submenu("alert-triangle-filled", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        let (icon, label) = if f.blocked {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("forbid-filled", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlock(id));
        ui.todo("flag-filled", aurora_i18n::tr!("common-report"));
    });
    ui.item("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.todo("coin-filled", aurora_i18n::tr!("common-pay"));
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
    ui.todo("code", aurora_i18n::tr!("ui-context-script-info"));
    ui.todo("eye-off", aurora_i18n::tr!("ui-context-derender-temporarily"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("photo-filled", aurora_i18n::tr!("ui-context-show-textures"));
    // the attachment's textures (Object.TexRefresh), not the avatar's bakes
    ui.item(
        "refresh",
        aurora_i18n::tr!("ui-context-refresh-textures"),
        CtxAction::RefreshTextures(attachment),
    );
    save_as_submenu(ui);
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
    ui.todo("shopping-cart-filled", aurora_i18n::tr!("ui-context-buy-this-land"));
    ui.todo("ticket-filled", aurora_i18n::tr!("ui-context-buy-access"));
    ui.separator();
    ui.item("box", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item("mountain-filled", aurora_i18n::tr!("ui-context-edit-land"), CtxAction::EditTerrain);
    ui.separator();
    ui.todo("sparkles-filled", aurora_i18n::tr!("ui-context-ignore-particle-owner"));
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
                "ui-context-add-to-contact-set",
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
            "common-delete",
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
}
