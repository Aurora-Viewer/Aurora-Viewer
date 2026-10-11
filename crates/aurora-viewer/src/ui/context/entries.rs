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
        Target::Avatar { id, attachment, .. } => avatar_menu(&mut entries, f, *id, *attachment, point),
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
        "hand-pointing",
        aurora_i18n::tr!("ui-context-touch"),
        o.touch,
        CtxAction::Touch(key.local_id),
    );
    ui.item("pencil-simple", aurora_i18n::tr!("common-edit"), CtxAction::Edit(id));
    ui.todo("paint-brush", aurora_i18n::tr!("ui-context-edit-pbr-material"));
    // Object.Build: the Create tool (LLToolCompCreate), like the land menu
    ui.item("cube", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item_if("package", aurora_i18n::tr!("common-open"), o.open, CtxAction::Open(id));
    ui.separator();
    ui.item_if(
        "armchair",
        aurora_i18n::tr!("ui-context-sit-here"),
        o.sit,
        CtxAction::Sit { target: id, offset },
    );
    ui.separator();
    ui.item_if("person", aurora_i18n::tr!("common-stand-up"), o.stand, CtxAction::StandUp);
    ui.separator();
    ui.submenu("cube", aurora_i18n::tr!("common-object"), true, |ui| {
        ui.todo("info", aurora_i18n::tr!("common-profile"));
        ui.todo("magnifying-glass", aurora_i18n::tr!("ui-context-inspect-2"));
        ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
        ui.separator();
        // linking needs two selected objects: build tools only
        ui.disabled("link", aurora_i18n::tr!("ui-context-link"));
        ui.item_if("link-break", aurora_i18n::tr!("ui-context-unlink"), o.unlink, CtxAction::Unlink(id));
        ui.check(
            "cube-transparent",
            aurora_i18n::tr!("ui-context-edit-linked-parts"),
            f.edit_linked,
            true,
            CtxAction::EditLinkedParts(!f.edit_linked),
        );
        ui.separator();
        ui.todo("arrows-clockwise", aurora_i18n::tr!("ui-context-reset-scripts"));
        ui.todo("play", aurora_i18n::tr!("ui-context-run-scripts"));
        ui.todo("stop-circle", aurora_i18n::tr!("ui-context-stop-scripts"));
        ui.todo("trash", aurora_i18n::tr!("ui-context-remove-scripts"));
    });
    ui.item("magnifying-glass-plus", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.separator();
    ui.submenu("t-shirt", aurora_i18n::tr!("ui-context-wear"), true, |ui| {
        ui.todo("t-shirt", aurora_i18n::tr!("common-wear"));
        ui.todo("plus-circle", aurora_i18n::tr!("common-add"));
        ui.todo("paperclip", aurora_i18n::tr!("common-attach-to"));
        ui.todo("paperclip", aurora_i18n::tr!("common-attach-to-hud"));
    });
    ui.submenu("warning", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        ui.todo("flag", aurora_i18n::tr!("common-report"));
        let (icon, label) = if o.blocked {
            ("check-circle", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("prohibit", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlockObject { id, name: o.name.clone() });
    });
    ui.separator();
    ui.todo("arrows-clockwise", aurora_i18n::tr!("ui-context-refresh-textures"));
    ui.todo("eye-slash", aurora_i18n::tr!("ui-context-derender"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("arrow-u-up-left", aurora_i18n::tr!("ui-context-return"));
    ui.item_if("hand-grabbing", aurora_i18n::tr!("ui-context-take"), o.take, CtxAction::Take(id));
    ui.item_if(
        "copy",
        aurora_i18n::tr!("ui-context-take-copy"),
        o.take_copy,
        CtxAction::TakeCopy(id),
    );
    ui.item_if("hand-coins", aurora_i18n::tr!("common-pay"), o.pay, CtxAction::Pay(id));
    ui.item_if("shopping-cart", aurora_i18n::tr!("common-buy"), o.buy, CtxAction::Buy(id));
    ui.todo("floppy-disk", aurora_i18n::tr!("common-save-as"));
    ui.separator();
    ui.item_if("trash", aurora_i18n::tr!("common-delete"), o.delete, CtxAction::Delete(id));
    ui.separator();
    ui.todo("sparkle", aurora_i18n::tr!("ui-context-ignore-particle-owner"));
}

/// menu_attachment_self.xml
fn attachment_self_menu(ui: &mut Builder, f: &Facts, key: ObjKey) {
    if let Some(scale) = f.object.local_hud_scale {
        ui.info(
            "magnifying-glass",
            &aurora_i18n::tr!("hud-local-scale", percent = format!("{:.0}", scale * 100.0)),
        );
        ui.item("arrow-counter-clockwise", aurora_i18n::tr!("hud-local-scale-reset"), CtxAction::ResetHudScale(key));
        ui.item_if(
            "eraser",
            aurora_i18n::tr!("hud-local-scale-forget"),
            f.object.saved_hud_scale,
            CtxAction::ForgetHudScale(key),
        );
        ui.separator();
    }
    ui.item_if(
        "hand-pointing",
        aurora_i18n::tr!("ui-context-touch"),
        f.object.touch,
        CtxAction::Touch(key.local_id),
    );
    ui.item_if(
        "backpack",
        aurora_i18n::tr!("ui-context-show-in-inventory"),
        f.object.attachment_item.is_some(),
        CtxAction::AttachmentInventory(f.object.attachment_item.unwrap_or_default()),
    );
    ui.item_if(
        "pencil-simple",
        aurora_i18n::tr!("common-edit"),
        f.object.edit_hud,
        CtxAction::EditAttachment(key),
    );
    ui.todo("paint-brush", aurora_i18n::tr!("ui-context-edit-pbr-material"));
    ui.item_if(
        "paperclip",
        aurora_i18n::tr!("common-detach"),
        f.object.attachment_item.is_some(),
        CtxAction::Detach(key),
    );
    ui.todo("floppy-disk", aurora_i18n::tr!("common-save-as"));
    ui.separator();
    sit_stand(ui, f);
    appearance_submenu(ui);
    take_off_submenu(ui);
    ui.separator();
    ui.item("airplane-takeoff", aurora_i18n::tr!("extra-fly-land-8"), CtxAction::ToggleFly);
    ui.todo("stop-circle", aurora_i18n::tr!("ui-context-stop-animations"));
    ui.separator();
    community_submenu(ui, f.me);
    ui.todo("info", aurora_i18n::tr!("ui-context-object-profile"));
    ui.todo("magnifying-glass", aurora_i18n::tr!("ui-context-inspect"));
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("eye-slash", aurora_i18n::tr!("ui-context-derender-temporarily"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("image", aurora_i18n::tr!("ui-context-show-textures"));
    ui.todo("arrows-clockwise", aurora_i18n::tr!("ui-context-refresh-textures"));
    ui.separator();
    // « Supprimer » drops the attachment on the ground (Attachment.Drop)
    ui.todo("trash", aurora_i18n::tr!("common-delete"));
    ui.separator();
    ui.todo("sparkle", aurora_i18n::tr!("ui-context-block-particle-owner"));
}

/// menu_avatar_other.xml / menu_attachment_other.xml
fn avatar_menu(ui: &mut Builder, f: &Facts, id: Uuid, attachment: Option<Uuid>, point: Vec3) {
    ui.item("user-circle", aurora_i18n::tr!("common-view-profile"), CtxAction::Profile(id));
    ui.item_if(
        "user-plus",
        aurora_i18n::tr!("common-become-friends"),
        !f.friend,
        CtxAction::AddFriend(id),
    );
    ui.todo("users-three", aurora_i18n::tr!("ui-context-add-to-contact-set"));
    ui.item("chat-text", aurora_i18n::tr!("ui-context-send-im"), CtxAction::Im(id));
    ui.todo("identification-card", aurora_i18n::tr!("ui-context-give-my-calling-card"));
    ui.todo("phone", aurora_i18n::tr!("common-voice-call"));
    ui.todo("user-circle-plus", aurora_i18n::tr!("ui-context-invite-to-group"));
    ui.item(
        "paper-plane-tilt",
        aurora_i18n::tr!("common-offer-teleport"),
        CtxAction::OfferTeleport(id),
    );
    ui.separator();
    ui.todo("eye", aurora_i18n::tr!("common-look-toward-avatar"));
    ui.separator();
    ui.todo("bone", aurora_i18n::tr!("ui-context-reset-skeleton"));
    ui.todo("person-simple-run", aurora_i18n::tr!("ui-context-reset-skeleton-and-animations"));
    ui.todo("arrow-counter-clockwise", aurora_i18n::tr!("ui-context-reset-mesh-lod-2"));
    ui.separator();
    ui.submenu("warning", aurora_i18n::tr!("ui-context-disturbance"), true, |ui| {
        let (icon, label) = if f.blocked {
            ("check-circle", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("prohibit", aurora_i18n::tr!("common-ignore"))
        };
        ui.item(icon, label, CtxAction::ToggleBlock(id));
        ui.todo("flag", aurora_i18n::tr!("common-report"));
        ui.todo("snowflake", aurora_i18n::tr!("ui-context-freeze"));
        ui.todo("door-open", aurora_i18n::tr!("ui-context-eject"));
    });
    ui.item("magnifying-glass-plus", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.todo("currency-circle-dollar", aurora_i18n::tr!("common-pay"));
    if attachment.is_some() {
        ui.separator();
        ui.todo("info", aurora_i18n::tr!("ui-context-object-profile"));
    }
    ui.separator();
    // Firestorm « Rendu toujours complet / Aucun rendu / Rendu selon vos préférences »
    if let Some((c, silhouette)) = f.complexity {
        let text = aurora_i18n::tr!(
            "ui-context-complexity",
            value0 = crate::ui::hud::group_digits(c).to_string(),
            value1 = (if silhouette { " (silhouette)" } else { "" }).to_string()
        );
        ui.info("gauge", &text);
    }
    for (mode, icon, label) in [
        (2u8, "user-focus", aurora_i18n::tr!("ui-context-always-show-full-detail")),
        (1, "ghost", aurora_i18n::tr!("ui-context-never-fully-render")),
        (0, "sliders", aurora_i18n::tr!("ui-context-display-according-to-your-preferences")),
    ] {
        ui.check(icon, label, f.render == mode, true, CtxAction::SetRender(id, mode));
    }
    ui.todo("magnifying-glass", aurora_i18n::tr!("ui-context-inspect-2"));
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("eye-slash", aurora_i18n::tr!("ui-context-derender-temporarily"));
    ui.todo("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"));
    ui.todo("image", aurora_i18n::tr!("ui-context-show-textures"));
    ui.todo("arrows-clockwise", aurora_i18n::tr!("ui-context-refresh-textures"));
    ui.separator();
    ui.todo("sparkle", aurora_i18n::tr!("common-block-particles"));
}

/// menu_avatar_self.xml
fn self_menu(ui: &mut Builder, f: &Facts, id: Uuid) {
    sit_stand(ui, f);
    take_off_submenu(ui);
    appearance_submenu(ui);
    community_submenu(ui, id);
    // not in Firestorm's menu: kept from Aurora's (fly is in menu_attachment_self.xml)
    ui.item(
        if f.flying { "airplane-landing" } else { "airplane-takeoff" },
        if f.flying {
            aurora_i18n::tr!("ui-context-land")
        } else {
            aurora_i18n::tr!("common-fly")
        },
        CtxAction::ToggleFly,
    );
    ui.item("camera", aurora_i18n::tr!("ui-context-reset-camera"), CtxAction::ResetCamera);
    ui.todo("code", aurora_i18n::tr!("ui-context-script-information"));
    ui.todo("image", aurora_i18n::tr!("ui-context-show-textures"));
    ui.todo("arrows-clockwise", aurora_i18n::tr!("ui-context-refresh-textures"));
}

/// « S'asseoir » (on the ground, Self.SitDown) and « Se lever ».
fn sit_stand(ui: &mut Builder, f: &Facts) {
    ui.item_if("armchair", aurora_i18n::tr!("status-sit"), !f.seated, CtxAction::SitGround);
    ui.item_if("person", aurora_i18n::tr!("common-stand-up"), f.seated, CtxAction::StandUp);
}

fn take_off_submenu(ui: &mut Builder) {
    ui.submenu("t-shirt", aurora_i18n::tr!("common-remove"), true, |ui| {
        ui.todo("t-shirt", aurora_i18n::tr!("common-clothing"));
        ui.todo("monitor", "HUD");
        ui.todo("paperclip", aurora_i18n::tr!("common-detach"));
        ui.todo("x-circle", aurora_i18n::tr!("ui-context-detach-all"));
    });
}

fn appearance_submenu(ui: &mut Builder) {
    ui.submenu("person", aurora_i18n::tr!("common-appearance"), true, |ui| {
        ui.item(
            "t-shirt",
            aurora_i18n::tr!("common-current-outfit"),
            CtxAction::OpenAppearance { tab: 2, editing: false },
        );
        ui.item(
            "backpack",
            aurora_i18n::tr!("ui-context-change-outfit"),
            CtxAction::OpenAppearance { tab: 0, editing: false },
        );
        ui.todo("person", aurora_i18n::tr!("ui-context-edit-shape"));
        ui.item(
            "pencil-simple",
            aurora_i18n::tr!("ui-context-edit-outfit"),
            CtxAction::OpenAppearance { tab: 2, editing: true },
        );
        ui.todo("arrows-out-cardinal", aurora_i18n::tr!("ui-context-flutter"));
        ui.separator();
        ui.todo("bone", aurora_i18n::tr!("ui-context-reset-skeleton"));
        ui.todo("person-simple-run", aurora_i18n::tr!("ui-context-reset-skeleton-and-animations"));
        ui.todo("arrow-counter-clockwise", aurora_i18n::tr!("ui-context-reset-mesh-lod"));
    });
}

fn community_submenu(ui: &mut Builder, me: Uuid) {
    ui.submenu("users-three", aurora_i18n::tr!("ui-context-community"), true, |ui| {
        ui.item("users", aurora_i18n::tr!("common-friends"), CtxAction::OpenPeople(1));
        ui.item("users-three", aurora_i18n::tr!("common-groups"), CtxAction::OpenPeople(2));
        ui.item("user-circle", aurora_i18n::tr!("common-profile"), CtxAction::Profile(me));
        ui.item(
            "identification-card",
            aurora_i18n::tr!("ui-context-display-name"),
            CtxAction::DisplayName,
        );
    });
}

/// menu_land.xml
fn land_menu(ui: &mut Builder, f: &Facts, point: Vec3) {
    ui.item("info", aurora_i18n::tr!("common-about-land"), CtxAction::AboutLand(point));
    // Land.Sit walks there first (autopilot): not in Aurora yet
    ui.todo("armchair", aurora_i18n::tr!("ui-context-sit-here"));
    if f.seated {
        ui.item("person", aurora_i18n::tr!("common-stand-up"), CtxAction::StandUp);
    }
    ui.item("magnifying-glass-plus", aurora_i18n::tr!("common-zoom-2"), CtxAction::Zoom(point));
    ui.separator();
    ui.todo("shopping-cart", aurora_i18n::tr!("ui-context-buy-this-land"));
    ui.todo("ticket", aurora_i18n::tr!("ui-context-buy-access"));
    ui.separator();
    ui.item("cube", aurora_i18n::tr!("common-build"), CtxAction::Build);
    ui.item("mountains", aurora_i18n::tr!("ui-context-edit-land"), CtxAction::EditTerrain);
    ui.separator();
    ui.todo("sparkle", aurora_i18n::tr!("ui-context-ignore-particle-owner"));
}
