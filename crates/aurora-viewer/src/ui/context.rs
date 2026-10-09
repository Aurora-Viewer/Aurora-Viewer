//! Right-click menus with Firestorm's entries and order: the world (land,
//! objects, our attachments, other avatars, our avatar: menu_land.xml,
//! menu_object.xml, menu_attachment_self.xml, menu_avatar_other.xml,
//! menu_attachment_other.xml, menu_avatar_self.xml) and the avatar and
//! group names of lists (menu_people_nearby.xml, menu_fs_contacts_friends.xml,
//! menu_url_agent.xml, menu_people_groups.xml), all drawn by ui/menu.rs.
//! Entries Aurora cannot do yet stay in place, greyed out with a tooltip,
//! the way Firestorm greys the ones that do not apply.
//!
//! Menus anywhere in the interface hand their choice to the app with
//! [`request`]; the app runs them with the world menu's ones.

use super::menu;
use crate::theme::Palette;
use crate::world::World;
use crate::world::objects::ObjKey;
use glam::Vec3;
use uuid::Uuid;

/// Object update flags (llprimitive/object_flags.h).
pub mod flags {
    pub const OBJECT_MODIFY: u32 = 1 << 2;
    pub const OBJECT_COPY: u32 = 1 << 3;
    pub const OBJECT_ANY_OWNER: u32 = 1 << 4;
    pub const OBJECT_YOU_OWNER: u32 = 1 << 5;
    pub const HANDLE_TOUCH: u32 = 1 << 7;
    pub const OBJECT_MOVE: u32 = 1 << 8;
    pub const TAKES_MONEY: u32 = 1 << 9;
    pub const OBJECT_TRANSFER: u32 = 1 << 17;
}

#[derive(Debug, Clone)]
pub enum Target {
    Ground,
    /// An object in the world.
    Object {
        key: ObjKey,
        full_id: Uuid,
        offset: Vec3,
    },
    /// One of our attachments (menu_attachment_self.xml).
    Attachment {
        key: ObjKey,
    },
    /// An avatar; `attachment` when one of its attachments was clicked
    /// (menu_attachment_other.xml adds « Profil de l'objet »).
    Avatar {
        id: Uuid,
        own: bool,
        attachment: Option<Uuid>,
    },
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub pos: egui::Pos2,
    /// World point that was clicked.
    pub point: Vec3,
    pub target: Target,
}

/// What the object menu entries depend on (the on_enable functions of
/// llviewermenu.cpp), worked out every frame while the menu is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectFacts {
    /// enable_object_touch: the prim or its parent handles touches.
    pub touch: bool,
    /// enable_object_sit: not already sitting on it.
    pub sit: bool,
    /// Object.EnableStandUp: we sit on this linkset.
    pub stand: bool,
    /// enable_object_open (LLViewerObject::allowOpen).
    pub open: bool,
    /// enable_take: ours, or modifiable and transferable; not sat on.
    pub take: bool,
    /// enable_object_take_copy: the root is copyable.
    pub take_copy: bool,
    /// enable_pay_object: the prim or its parent takes money.
    pub pay: bool,
    /// enable_buy_object: for sale and not ours (once ObjectProperties came).
    pub buy: bool,
    /// LLSelectMgr::canDoDelete (getFirstDeleteableObject).
    pub delete: bool,
    /// LLSelectMgr::enableUnlinkObjects: a linkset we can modify.
    pub unlink: bool,
    /// Blocked (LLMuteList), for « Ignorer » / « Cesser d'ignorer ».
    pub blocked: bool,
    /// Name from ObjectProperties (block list entry).
    pub name: String,
}

impl ObjectFacts {
    /// The rules on the update flags of the clicked prim, its root and all
    /// the prims of the linkset.
    pub fn from_flags(clicked: u32, parent: u32, root: u32, family: &[u32], sitting_on: bool, for_sale: bool) -> ObjectFacts {
        let has = |f: u32, bit: u32| f & bit != 0;
        let you_own = has(root, flags::OBJECT_YOU_OWNER);
        ObjectFacts {
            touch: has(clicked, flags::HANDLE_TOUCH) || has(parent, flags::HANDLE_TOUCH),
            sit: !sitting_on,
            stand: sitting_on,
            take: !sitting_on && (you_own || has(root, flags::OBJECT_MODIFY) && has(root, flags::OBJECT_TRANSFER)),
            take_copy: has(root, flags::OBJECT_COPY),
            pay: has(clicked, flags::TAKES_MONEY) || has(parent, flags::TAKES_MONEY),
            buy: for_sale && !you_own && has(root, flags::OBJECT_ANY_OWNER),
            // you can delete what you own or may modify, or what no one owns
            delete: family
                .iter()
                .any(|&f| has(f, flags::OBJECT_MODIFY) || has(f, flags::OBJECT_YOU_OWNER) || !has(f, flags::OBJECT_ANY_OWNER)),
            unlink: family.len() > 1 && has(root, flags::OBJECT_MODIFY),
            ..ObjectFacts::default()
        }
    }
}

/// The rest of what the menus show, refreshed every frame.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Our agent id.
    pub me: Uuid,
    pub seated: bool,
    pub flying: bool,
    pub object: ObjectFacts,
    /// Avatar: (complexity, shown as a silhouette).
    pub complexity: Option<(u32, bool)>,
    /// Rendering exception: 0 normal, 1 never fully, 2 always fully.
    pub render: u8,
    pub blocked: bool,
    pub friend: bool,
    /// EditLinkedParts.
    pub edit_linked: bool,
}

#[derive(Debug, Clone)]
pub enum CtxAction {
    Touch(u32),
    Sit {
        target: Uuid,
        offset: Vec3,
    },
    Zoom(Vec3),
    /// About Land on the parcel at the clicked point (LLToolPie selects it).
    AboutLand(Vec3),
    StandUp,
    SitGround,
    ToggleFly,
    ResetCamera,
    Im(Uuid),
    OfferTeleport(Uuid),
    Profile(Uuid),
    /// Rendering exception of an avatar (LLRenderMuteList).
    SetRender(Uuid, u8),
    /// Block / unblock a resident (LLMuteList).
    ToggleBlock(Uuid),
    /// Block / unblock an object (LLMute::OBJECT).
    ToggleBlockObject {
        id: Uuid,
        name: String,
    },
    /// Edit the object in the build tools (handle_object_edit).
    Edit(Uuid),
    /// Take / take a copy into the inventory (derez_objects).
    Take(Uuid),
    TakeCopy(Uuid),
    /// Delete to the trash (LLSelectMgr::selectDelete), asking first when
    /// it is locked, not copyable or not ours.
    Delete(Uuid),
    DeleteConfirmed(Uuid),
    /// Pay / buy / open the object (the click-action dialogs).
    Pay(Uuid),
    Buy(Uuid),
    Open(Uuid),
    /// Unlink the linkset (Tools.Unlink).
    Unlink(Uuid),
    /// « Modifier les parties liées » (Tools.EditLinkedParts).
    EditLinkedParts(bool),
    /// Land menu: build tools, create (Land.Build) or terraform (Land.Edit).
    Build,
    EditTerrain,
    /// Change our display name (LLFloaterDisplayName).
    DisplayName,
    OpenAppearance {
        tab: usize,
        editing: bool,
    },
    /// People floater on a tab (0 nearby, 1 friends, 2 groups).
    OpenPeople(u8),
    /// Camera on an avatar of a list (Avatar.ZoomIn).
    ZoomAvatar(Uuid),
    /// Teleport to where an avatar is (FSRadar teleport_to).
    TeleportToAvatar(Uuid),
    /// World map on an avatar (Avatar.ShowOnMap).
    ShowOnMap(Uuid),
    /// Place link of a text (SLURL): place details window (FSFloaterPlaceDetails
    /// "remote_place"), world map on it, teleport there once confirmed
    /// (TeleportViaSLAPP).
    ShowPlaceInfo(String, glam::Vec3),
    ShowPlace(String, glam::Vec3),
    /// Web link clicked in a text: opened at once when trusted, else after
    /// the external link warning.
    OpenUrl(String),
    TeleportToPlace(String, glam::Vec3),
    TeleportToPlaceConfirmed(String, glam::Vec3),
    /// Rights given to a friend (GrantUserRights).
    GrantRights {
        friend: Uuid,
        rights: i32,
    },
    /// Friendship: offer (with a message) / end, both confirmed first.
    AddFriend(Uuid),
    RemoveFriend(Uuid),
    /// Ask an avatar to teleport us to them (TeleportRequest prompt).
    RequestTeleport(Uuid),
    GroupChat(Uuid),
    /// Make a group active (ActivateGroup), pin it in the lists, leave it.
    ActivateGroup(Uuid),
    PinGroup(Uuid),
    LeaveGroup(Uuid),
    /// Block / unblock a group's chat (exoGroupMuteList).
    GroupChatBlocked(Uuid, bool),
}

/// egui temp data: actions chosen in menus this frame.
fn requests_id() -> egui::Id {
    egui::Id::new("aurora_context_requests")
}

/// Hand a menu choice to the app (run after the interface pass).
pub fn request(ctx: &egui::Context, a: CtxAction) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<CtxAction>>(requests_id()).push(a));
}

pub fn take_requests(ctx: &egui::Context) -> Vec<CtxAction> {
    ctx.data_mut(|d| d.remove_temp::<Vec<CtxAction>>(requests_id())).unwrap_or_default()
}

/// Where an avatar is (render space): its object if loaded, else its
/// coarse location (the radar's positions).
pub fn avatar_position(world: &World, id: &Uuid) -> Option<Vec3> {
    if let Some(o) = world.objects.index_of_uuid(id).and_then(|i| world.objects.get(i))
        && let Some(off) = world.region_offset(o.key.region)
    {
        return Some(off + o.position);
    }
    world
        .coarse
        .iter()
        .find_map(|(h, list)| Some(world.region_offset(*h)? + *list.iter().find(|(i, _)| i == id).map(|(_, p)| p)?))
}

/// egui temp data: row hovered last frame in the world menu.
const HOVERED: &str = "world_context_menu_hovered";

/// Draw the world menu; `menu` becomes `None` once it closed.
pub fn show(ctx: &egui::Context, p: &Palette, menu: &mut Option<ContextMenu>, f: &Facts) {
    let Some(m) = menu.clone() else {
        return;
    };
    let mut open = true;
    let hovered = menu::popup_at(ctx, egui::Id::new("world_context_menu"), m.pos, &mut open, p, |ui| {
        let ctx = ui.ctx().clone();
        let mut act = |a: CtxAction| request(&ctx, a);
        match &m.target {
            Target::Object { key, full_id, offset } => object_menu(ui, p, f, *key, *full_id, *offset, m.point, &mut act),
            Target::Attachment { key } => attachment_self_menu(ui, p, f, *key, &mut act),
            Target::Avatar { id, own: true, .. } => self_menu(ui, p, f, *id, &mut act),
            Target::Avatar { id, attachment, .. } => avatar_menu(ui, p, f, *id, *attachment, m.point, &mut act),
            Target::Ground => land_menu(ui, p, f, m.point, &mut act),
        }
    });
    // UISndPieMenuSliceHighlight0..7 when an enabled row gets hovered (the
    // eight slices of Firestorm's default pie menu, PieMenu::draw)
    let before = ctx.data_mut(|d| d.get_temp::<Option<usize>>(egui::Id::new(HOVERED)).flatten());
    if hovered != before {
        if let Some(i) = hovered.filter(|i| *i < 8) {
            super::sound_cues::request(ctx, crate::ui_sound::UiSound::PieMenuSlice(i as u8));
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(HOVERED), hovered));
    }
    if !open {
        *menu = None;
        ctx.data_mut(|d| d.remove::<Option<usize>>(egui::Id::new(HOVERED)));
    }
}

/// menu_object.xml
#[allow(clippy::too_many_arguments)]
fn object_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    f: &Facts,
    key: ObjKey,
    id: Uuid,
    offset: Vec3,
    point: Vec3,
    act: &mut dyn FnMut(CtxAction),
) {
    let o = &f.object;
    if menu::item_if(ui, p, "hand-pointing", "Toucher", o.touch) {
        act(CtxAction::Touch(key.local_id));
    }
    if menu::item(ui, p, "pencil-simple", "Modifier") {
        act(CtxAction::Edit(id));
    }
    menu::todo(ui, p, "paint-brush", "Modifier le matériau PBR");
    // Object.Build: the Create tool (LLToolCompCreate), like the land menu
    if menu::item(ui, p, "cube", "Construire") {
        act(CtxAction::Build);
    }
    if menu::item_if(ui, p, "package", "Ouvrir", o.open) {
        act(CtxAction::Open(id));
    }
    menu::separator(ui, p);
    if menu::item_if(ui, p, "armchair", "S'asseoir ici", o.sit) {
        act(CtxAction::Sit { target: id, offset });
    }
    menu::separator(ui, p);
    if menu::item_if(ui, p, "person", "Se lever", o.stand) {
        act(CtxAction::StandUp);
    }
    menu::separator(ui, p);
    menu::submenu(ui, p, "cube", "Objet", true, |ui| {
        menu::todo(ui, p, "info", "Profil");
        menu::todo(ui, p, "magnifying-glass", "Inspecter");
        menu::todo(ui, p, "code", "Informations sur les scripts");
        menu::separator(ui, p);
        // linking needs two selected objects: build tools only
        menu::item_if(ui, p, "link", "Lier", false);
        if menu::item_if(ui, p, "link-break", "Délier", o.unlink) {
            act(CtxAction::Unlink(id));
        }
        if menu::check(ui, p, "cube-transparent", "Modifier les parties liées", f.edit_linked, true) {
            act(CtxAction::EditLinkedParts(!f.edit_linked));
        }
        menu::separator(ui, p);
        menu::todo(ui, p, "arrows-clockwise", "Réinitialiser les scripts");
        menu::todo(ui, p, "play", "Exécuter les scripts");
        menu::todo(ui, p, "stop-circle", "Arrêter les scripts");
        menu::todo(ui, p, "trash", "Supprimer les scripts");
    });
    if menu::item(ui, p, "magnifying-glass-plus", "Zoomer") {
        act(CtxAction::Zoom(point));
    }
    menu::separator(ui, p);
    menu::submenu(ui, p, "t-shirt", "Mettre", true, |ui| {
        menu::todo(ui, p, "t-shirt", "Porter");
        menu::todo(ui, p, "plus-circle", "Ajouter");
        menu::todo(ui, p, "paperclip", "Attacher à");
        menu::todo(ui, p, "paperclip", "Attacher au HUD");
    });
    menu::submenu(ui, p, "warning", "Dérangement", true, |ui| {
        menu::todo(ui, p, "flag", "Signaler");
        let (icon, label) = if o.blocked {
            ("check-circle", "Cesser d'ignorer")
        } else {
            ("prohibit", "Ignorer")
        };
        if menu::item(ui, p, icon, label) {
            act(CtxAction::ToggleBlockObject { id, name: o.name.clone() });
        }
    });
    menu::separator(ui, p);
    menu::todo(ui, p, "arrows-clockwise", "Actualiser les textures");
    menu::todo(ui, p, "eye-slash", "Faire disparaître");
    menu::todo(ui, p, "eye-closed", "Faire disparaître & Blacklister");
    menu::todo(ui, p, "arrow-u-up-left", "Renvoyer");
    if menu::item_if(ui, p, "hand-grabbing", "Prendre", o.take) {
        act(CtxAction::Take(id));
    }
    if menu::item_if(ui, p, "copy", "Prendre une copie", o.take_copy) {
        act(CtxAction::TakeCopy(id));
    }
    if menu::item_if(ui, p, "hand-coins", "Payer", o.pay) {
        act(CtxAction::Pay(id));
    }
    if menu::item_if(ui, p, "shopping-cart", "Acheter", o.buy) {
        act(CtxAction::Buy(id));
    }
    menu::todo(ui, p, "floppy-disk", "Enregistrer sous");
    menu::separator(ui, p);
    if menu::item_if(ui, p, "trash", "Supprimer", o.delete) {
        act(CtxAction::Delete(id));
    }
    menu::separator(ui, p);
    menu::todo(ui, p, "sparkle", "Ignorer le propriétaire des particules");
}

/// menu_attachment_self.xml
fn attachment_self_menu(ui: &mut egui::Ui, p: &Palette, f: &Facts, key: ObjKey, act: &mut dyn FnMut(CtxAction)) {
    if menu::item_if(ui, p, "hand-pointing", "Toucher", f.object.touch) {
        act(CtxAction::Touch(key.local_id));
    }
    menu::todo(ui, p, "backpack", "Voir dans l'inventaire");
    menu::todo(ui, p, "pencil-simple", "Modifier");
    menu::todo(ui, p, "paint-brush", "Modifier le matériau PBR");
    menu::todo(ui, p, "paperclip", "Détacher");
    menu::todo(ui, p, "floppy-disk", "Enregistrer sous");
    menu::separator(ui, p);
    sit_stand(ui, p, f, act);
    appearance_submenu(ui, p, act);
    take_off_submenu(ui, p);
    menu::separator(ui, p);
    if menu::item(ui, p, "airplane-takeoff", "Voler / Atterrir") {
        act(CtxAction::ToggleFly);
    }
    menu::todo(ui, p, "stop-circle", "Arrêter les animations");
    menu::separator(ui, p);
    community_submenu(ui, p, f.me, act);
    menu::todo(ui, p, "info", "Profil de l'objet");
    menu::todo(ui, p, "magnifying-glass", "Examiner");
    menu::todo(ui, p, "code", "Informations sur les scripts");
    menu::todo(ui, p, "eye-slash", "Faire disparaître temporairement");
    menu::todo(ui, p, "eye-closed", "Faire disparaître & Blacklister");
    menu::todo(ui, p, "image", "Afficher les textures");
    menu::todo(ui, p, "arrows-clockwise", "Actualiser les textures");
    menu::separator(ui, p);
    // « Supprimer » drops the attachment on the ground (Attachment.Drop)
    menu::todo(ui, p, "trash", "Supprimer");
    menu::separator(ui, p);
    menu::todo(ui, p, "sparkle", "Bloquer le propriétaire des particules");
}

/// menu_avatar_other.xml / menu_attachment_other.xml
fn avatar_menu(ui: &mut egui::Ui, p: &Palette, f: &Facts, id: Uuid, attachment: Option<Uuid>, point: Vec3, act: &mut dyn FnMut(CtxAction)) {
    if menu::item(ui, p, "user-circle", "Voir le profil") {
        act(CtxAction::Profile(id));
    }
    if menu::item_if(ui, p, "user-plus", "Devenir amis", !f.friend) {
        act(CtxAction::AddFriend(id));
    }
    menu::todo(ui, p, "users-three", "Ajouter à un cercle");
    if menu::item(ui, p, "chat-text", "Envoyer un IM") {
        act(CtxAction::Im(id));
    }
    menu::todo(ui, p, "identification-card", "Donner ma carte de visite");
    menu::todo(ui, p, "phone", "Appel vocal");
    menu::todo(ui, p, "user-circle-plus", "Inviter dans un groupe");
    if menu::item(ui, p, "paper-plane-tilt", "Proposer une téléportation") {
        act(CtxAction::OfferTeleport(id));
    }
    menu::separator(ui, p);
    menu::todo(ui, p, "eye", "Regard vers l'avatar");
    menu::separator(ui, p);
    menu::todo(ui, p, "bone", "Réinitialiser le squelette");
    menu::todo(ui, p, "person-simple-run", "Réinitialiser le squelette et les animations");
    menu::todo(ui, p, "arrow-counter-clockwise", "Réinitialiser le LOD du mesh");
    menu::separator(ui, p);
    menu::submenu(ui, p, "warning", "Dérangement", true, |ui| {
        let (icon, label) = if f.blocked {
            ("check-circle", "Cesser d'ignorer")
        } else {
            ("prohibit", "Ignorer")
        };
        if menu::item(ui, p, icon, label) {
            act(CtxAction::ToggleBlock(id));
        }
        menu::todo(ui, p, "flag", "Signaler");
        menu::todo(ui, p, "snowflake", "Geler");
        menu::todo(ui, p, "door-open", "Expulser");
    });
    if menu::item(ui, p, "magnifying-glass-plus", "Zoomer") {
        act(CtxAction::Zoom(point));
    }
    menu::todo(ui, p, "currency-circle-dollar", "Payer");
    if attachment.is_some() {
        menu::separator(ui, p);
        menu::todo(ui, p, "info", "Profil de l'objet");
    }
    menu::separator(ui, p);
    // Firestorm « Rendu toujours complet / Aucun rendu / Rendu selon vos préférences »
    if let Some((c, silhouette)) = f.complexity {
        let text = format!(
            "Complexité : {}{}",
            super::hud::group_digits(c),
            if silhouette { " (silhouette)" } else { "" }
        );
        menu::info(ui, p, "gauge", &text);
    }
    for (mode, icon, label) in [
        (2u8, "user-focus", "Toujours afficher tous les détails"),
        (1, "ghost", "Ne jamais afficher tous les détails"),
        (0, "sliders", "Affichage selon vos préférences"),
    ] {
        if menu::check(ui, p, icon, label, f.render == mode, true) {
            act(CtxAction::SetRender(id, mode));
        }
    }
    menu::todo(ui, p, "magnifying-glass", "Inspecter");
    menu::todo(ui, p, "code", "Informations sur les scripts");
    menu::todo(ui, p, "eye-slash", "Faire disparaître temporairement");
    menu::todo(ui, p, "eye-closed", "Faire disparaître & Blacklister");
    menu::todo(ui, p, "image", "Afficher les textures");
    menu::todo(ui, p, "arrows-clockwise", "Actualiser les textures");
    menu::separator(ui, p);
    menu::todo(ui, p, "sparkle", "Bloquer les particules");
}

/// menu_avatar_self.xml
fn self_menu(ui: &mut egui::Ui, p: &Palette, f: &Facts, id: Uuid, act: &mut dyn FnMut(CtxAction)) {
    sit_stand(ui, p, f, act);
    take_off_submenu(ui, p);
    appearance_submenu(ui, p, act);
    community_submenu(ui, p, id, act);
    // not in Firestorm's menu: kept from Aurora's (fly is in menu_attachment_self.xml)
    if menu::item(
        ui,
        p,
        if f.flying { "airplane-landing" } else { "airplane-takeoff" },
        if f.flying { "Atterrir" } else { "Voler" },
    ) {
        act(CtxAction::ToggleFly);
    }
    if menu::item(ui, p, "camera", "Réinitialiser la caméra") {
        act(CtxAction::ResetCamera);
    }
    menu::todo(ui, p, "code", "Informations sur les scripts");
    menu::todo(ui, p, "image", "Afficher les textures");
    menu::todo(ui, p, "arrows-clockwise", "Actualiser les textures");
}

/// « S'asseoir » (on the ground, Self.SitDown) and « Se lever ».
fn sit_stand(ui: &mut egui::Ui, p: &Palette, f: &Facts, act: &mut dyn FnMut(CtxAction)) {
    if menu::item_if(ui, p, "armchair", "S'asseoir", !f.seated) {
        act(CtxAction::SitGround);
    }
    if menu::item_if(ui, p, "person", "Se lever", f.seated) {
        act(CtxAction::StandUp);
    }
}

fn take_off_submenu(ui: &mut egui::Ui, p: &Palette) {
    menu::submenu(ui, p, "t-shirt", "Enlever", true, |ui| {
        menu::todo(ui, p, "t-shirt", "Vêtements");
        menu::todo(ui, p, "monitor", "HUD");
        menu::todo(ui, p, "paperclip", "Détacher");
        menu::todo(ui, p, "x-circle", "Tout détacher");
    });
}

fn appearance_submenu(ui: &mut egui::Ui, p: &Palette, act: &mut dyn FnMut(CtxAction)) {
    menu::submenu(ui, p, "person", "Apparence", true, |ui| {
        if menu::item(ui, p, "t-shirt", "Tenue actuelle") {
            act(CtxAction::OpenAppearance { tab: 2, editing: false });
        }
        if menu::item(ui, p, "backpack", "Changer de tenue") {
            act(CtxAction::OpenAppearance { tab: 0, editing: false });
        }
        menu::todo(ui, p, "person", "Modifier la silhouette");
        if menu::item(ui, p, "pencil-simple", "Modifier la tenue") {
            act(CtxAction::OpenAppearance { tab: 2, editing: true });
        }
        menu::todo(ui, p, "arrows-out-cardinal", "Voltigement");
        menu::separator(ui, p);
        menu::todo(ui, p, "bone", "Réinitialiser le squelette");
        menu::todo(ui, p, "person-simple-run", "Réinitialiser le squelette et les animations");
        menu::todo(ui, p, "arrow-counter-clockwise", "Réinitialiser le LOD du maillage");
    });
}

fn community_submenu(ui: &mut egui::Ui, p: &Palette, me: Uuid, act: &mut dyn FnMut(CtxAction)) {
    let mut chosen = None;
    menu::submenu(ui, p, "users-three", "Communauté", true, |ui| {
        if menu::item(ui, p, "users", "Amis") {
            chosen = Some(CtxAction::OpenPeople(1));
        }
        if menu::item(ui, p, "users-three", "Groupes") {
            chosen = Some(CtxAction::OpenPeople(2));
        }
        if menu::item(ui, p, "user-circle", "Profil") {
            chosen = Some(CtxAction::Profile(me));
        }
        if menu::item(ui, p, "identification-card", "Nom d'affichage…") {
            chosen = Some(CtxAction::DisplayName);
        }
    });
    if let Some(a) = chosen {
        act(a);
    }
}

/// menu_land.xml
fn land_menu(ui: &mut egui::Ui, p: &Palette, f: &Facts, point: Vec3, act: &mut dyn FnMut(CtxAction)) {
    if menu::item(ui, p, "info", "À propos du terrain") {
        act(CtxAction::AboutLand(point));
    }
    // Land.Sit walks there first (autopilot): not in Aurora yet
    menu::todo(ui, p, "armchair", "S'asseoir ici");
    if f.seated && menu::item(ui, p, "person", "Se lever") {
        act(CtxAction::StandUp);
    }
    if menu::item(ui, p, "magnifying-glass-plus", "Zoomer") {
        act(CtxAction::Zoom(point));
    }
    menu::separator(ui, p);
    menu::todo(ui, p, "shopping-cart", "Acheter ce terrain");
    menu::todo(ui, p, "ticket", "Acheter un droit d'entrée");
    menu::separator(ui, p);
    if menu::item(ui, p, "cube", "Construire") {
        act(CtxAction::Build);
    }
    if menu::item(ui, p, "mountains", "Modifier le terrain") {
        act(CtxAction::EditTerrain);
    }
    menu::separator(ui, p);
    menu::todo(ui, p, "sparkle", "Ignorer le propriétaire des particules");
}

/// Where an avatar menu of a list is opened: each place has its own
/// Firestorm menu, with the same entries in a different order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarList {
    /// People › Nearby (menu_people_nearby.xml) and the mini-map.
    Nearby,
    /// People › Friends (menu_fs_contacts_friends.xml).
    Friend,
    /// A name in the chat or a conversation (menu_url_agent.xml).
    Name,
}

/// Right-click menu of an avatar name in a list. Our own name gets only
/// the profile.
pub fn avatar_list_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, kind: AvatarList) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| request(&ctx, a);
    if id == world.agent_id {
        if menu::item(ui, p, "user-circle", "Voir le profil") {
            act(CtxAction::Profile(id));
        }
        return;
    }
    let friend = world.social.friends.iter().find(|f| f.id == id).cloned();
    let blocked = world.is_avatar_blocked(&id);
    let here = avatar_position(world, &id).is_some();
    let online = friend.as_ref().is_none_or(|f| f.online);
    let name = world.social.name_of(&id);
    let profile = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "user-circle", "Voir le profil") {
            act(CtxAction::Profile(id));
        }
    };
    let im = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "chat-text", "Envoyer un IM") {
            act(CtxAction::Im(id));
        }
    };
    let offer_tp = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "paper-plane-tilt", "Proposer une téléportation", online) {
            act(CtxAction::OfferTeleport(id));
        }
    };
    let tp_to = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "navigation-arrow", "Se téléporter vers", here) {
            act(CtxAction::TeleportToAvatar(id));
        }
    };
    let zoom = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "magnifying-glass-plus", "Zoomer", here) {
            act(CtxAction::ZoomAvatar(id));
        }
    };
    let block = |ui: &mut egui::Ui| {
        if menu::check(ui, p, "prohibit", "Ignorer", blocked, true) {
            act(CtxAction::ToggleBlock(id));
        }
    };
    let copy = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "copy", "Copier le nom") {
            ui.ctx().copy_text(name.clone());
        }
        if menu::item(ui, p, "link", "Copier l'URL") {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
        }
        if menu::item(ui, p, "at", "Copier l'URI de la mention") {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/mention"));
        }
    };
    let add_friend = |ui: &mut egui::Ui| {
        if friend.is_none() && menu::item(ui, p, "user-plus", "Devenir amis") {
            act(CtxAction::AddFriend(id));
        }
    };
    let remove_friend = |ui: &mut egui::Ui| {
        if friend.is_some() && menu::item(ui, p, "user-minus", "Supprimer cet ami") {
            act(CtxAction::RemoveFriend(id));
        }
    };
    let request_tp = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "airplane-landing", "Demander une téléportation", online) {
            act(CtxAction::RequestTeleport(id));
        }
    };
    match kind {
        AvatarList::Nearby => {
            profile(ui);
            im(ui);
            offer_tp(ui);
            request_tp(ui);
            tp_to(ui);
            menu::todo(ui, p, "phone", "Appel vocal");
            menu::separator(ui, p);
            menu::todo(ui, p, "clock-counter-clockwise", "Voir l'historique de conversations");
            menu::separator(ui, p);
            add_friend(ui);
            remove_friend(ui);
            menu::todo(ui, p, "users-three", "Ajouter à un cercle");
            menu::todo(ui, p, "user-circle-plus", "Inviter dans un groupe");
            menu::separator(ui, p);
            zoom(ui);
            if menu::item_if(ui, p, "map-trifold", "Voir sur la carte", here) {
                act(CtxAction::ShowOnMap(id));
            }
            menu::todo(ui, p, "share-network", "Partager");
            menu::todo(ui, p, "currency-circle-dollar", "Payer");
            block(ui);
        }
        AvatarList::Friend => {
            profile(ui);
            im(ui);
            menu::todo(ui, p, "clock-counter-clockwise", "Voir l'historique de conversations");
            menu::todo(ui, p, "users-three", "Ajouter au cercle");
            zoom(ui);
            tp_to(ui);
            offer_tp(ui);
            request_tp(ui);
            menu::todo(ui, p, "currency-circle-dollar", "Payer");
            menu::todo(ui, p, "crosshair", "Suivre");
            remove_friend(ui);
            menu::separator(ui, p);
            copy(ui);
            if let Some(f) = &friend {
                menu::separator(ui, p);
                // GlobalOnlineStatusToggle: the online status right we give
                let on = f.rights_given & 1 != 0;
                if menu::check(ui, p, "eye", "Statut connecté visible pour cet ami", on, true) {
                    act(CtxAction::GrantRights {
                        friend: id,
                        rights: f.rights_given ^ 1,
                    });
                }
            }
        }
        AvatarList::Name => {
            profile(ui);
            im(ui);
            menu::todo(ui, p, "clock-counter-clockwise", "Voir l'historique de conversations");
            add_friend(ui);
            menu::todo(ui, p, "users-three", "Ajouter à un cercle");
            zoom(ui);
            tp_to(ui);
            offer_tp(ui);
            request_tp(ui);
            menu::todo(ui, p, "crosshair", "Suivre le résident");
            remove_friend(ui);
            menu::separator(ui, p);
            menu::todo(ui, p, "flag", "Signaler une infraction");
            block(ui);
            menu::separator(ui, p);
            copy(ui);
        }
    }
}

/// Right-click menu of one of our groups (menu_people_groups.xml), plus
/// Aurora's « Bloquer le chat du groupe » (exoGroupMuteList).
pub fn group_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| request(&ctx, a);
    if menu::item_if(ui, p, "check-circle", "Activer", world.groups.active != id) {
        act(CtxAction::ActivateGroup(id));
    }
    menu::todo(ui, p, "info", "Voir les infos");
    if menu::item(ui, p, "link", "Copier le SLurl") {
        ui.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
    }
    if menu::item(ui, p, "chat-text", "Chat") {
        act(CtxAction::GroupChat(id));
    }
    menu::todo(ui, p, "phone", "Appel vocal");
    let pin = if world.groups.favorites.contains(&id) {
        "Désépingler le groupe"
    } else {
        "Épingler le groupe"
    };
    if menu::item(ui, p, "push-pin", pin) {
        act(CtxAction::PinGroup(id));
    }
    let blocked = world.mutes.group_chat_muted(&id);
    if menu::check(ui, p, "chat-teardrop-slash", "Bloquer le chat du groupe", blocked, true) {
        act(CtxAction::GroupChatBlocked(id, !blocked));
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "sign-out", "Quitter") {
        act(CtxAction::LeaveGroup(id));
    }
}

/// The avatar wearing object `idx`, if it is an attachment.
pub fn wearer(world: &World, idx: usize) -> Option<Uuid> {
    let mut o = world.objects.get(idx)?;
    for _ in 0..64 {
        if o.parent_id == 0 {
            return None;
        }
        let parent = world.objects.get(world.objects.parent_of(o)?)?;
        if parent.is_avatar() {
            return Some(parent.full_id);
        }
        o = parent;
    }
    None
}

/// LLSelectMgr::selectDelete: deleting something locked, not copyable or
/// not ours asks first (ConfirmObjectDelete* of notifications.xml, French
/// skin). `flags`: the update flags of every prim deleted.
pub fn delete_warning(flags: &[u32]) -> Option<String> {
    let locked = flags.iter().any(|f| f & flags::OBJECT_MOVE == 0);
    let no_copy = flags.iter().any(|f| f & flags::OBJECT_COPY == 0);
    let not_ours = flags.iter().any(|f| f & flags::OBJECT_YOU_OWNER == 0);
    let mut lines = Vec::new();
    if locked {
        lines.push("Au moins un des objets est verrouillé.");
    }
    if no_copy {
        lines.push("Au moins un des objets n'est pas copiable.");
    }
    if not_ours {
        lines.push("Au moins un des objets ne vous appartient pas.");
    }
    (!lines.is_empty()).then(|| {
        format!(
            "{}

Êtes-vous certain de vouloir supprimer ces objets ?",
            lines.join(
                "
"
            )
        )
    })
}

/// The delete confirmation; Some(true) to delete, Some(false) to cancel.
pub fn delete_confirm(ctx: &egui::Context, p: &Palette, text: &str) -> Option<bool> {
    use egui::{Color32, CornerRadius, RichText};
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("object_delete_confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new("Supprimer l'objet ?").size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(12.5).color(p.muted));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let yes = ui.add(
                egui::Button::new(RichText::new("Supprimer").color(Color32::WHITE))
                    .fill(p.danger)
                    .corner_radius(CornerRadius::same(2)),
            );
            if yes.clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, "Annuler").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// TeleportViaSLAPP: confirm a teleport asked by a place link. Some(true)
/// teleports, Some(false) cancels.
pub fn place_teleport_confirm(ctx: &egui::Context, p: &Palette, region: &str) -> Option<bool> {
    use egui::RichText;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("place_teleport_confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new("Téléportation").size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Voulez-vous vraiment vous téléporter jusqu'à {region} ?"))
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if super::widgets::flat_button(ui, p, "Téléporter").clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, "Annuler").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// Warning before opening a web link outside the trusted domains (phishing,
/// fake login pages...). Some(true) opens it, Some(false) cancels;
/// `dont_warn` is the « Ne plus me prévenir » box. A dangerous link
/// (`danger`: the reason, from `link_trust`) gets the red version, without
/// that box: it always warns.
pub fn external_link_confirm(ctx: &egui::Context, p: &Palette, url: &str, dont_warn: &mut bool, danger: Option<&str>) -> Option<bool> {
    use egui::RichText;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("external_link_confirm")).show(ctx, |ui| {
        ui.set_width(380.0);
        let (icon, col, title) = match danger {
            Some(_) => ("x-circle-fill", p.danger, "Lien dangereux"),
            None => ("warning-fill", p.amber, "Attention avant de cliquer"),
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            if let Some(t) = super::icons::global(icon) {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), rect, uv, col);
            }
            ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
        });
        ui.add_space(6.0);
        if let Some(why) = danger {
            ui.label(RichText::new(why).size(12.5).strong().color(p.danger));
            ui.add_space(4.0);
        }
        let text = match danger {
            Some(_) => {
                "Aurora vous déconseille d'ouvrir ce lien. S'il vous demande de vous connecter ou de \
                 payer, c'est très probablement une arnaque : ne donnez jamais votre mot de passe ni \
                 vos informations de paiement."
            }
            None => {
                "Ce lien mène à un site qui n'est pas dans la liste des sites de confiance d'Aurora. \
                 Méfiez-vous des fausses pages de connexion et des offres trop belles : ne donnez \
                 jamais votre mot de passe ni vos informations de paiement."
            }
        };
        ui.label(RichText::new(text).size(12.5).color(p.muted));
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(RichText::new(url).size(12.0).monospace().color(p.ink))
                .wrap()
                .selectable(true),
        );
        ui.add_space(8.0);
        if danger.is_none() {
            ui.checkbox(dont_warn, RichText::new("Ne plus me prévenir").size(12.5).color(p.muted));
            ui.add_space(8.0);
        }
        // the action, then « Annuler », like the other confirmations
        ui.horizontal(|ui| {
            let visit = if danger.is_some() {
                "Visiter quand même"
            } else {
                "Visiter le lien"
            };
            if super::widgets::flat_button(ui, p, visit).clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, "Annuler").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINE: u32 = flags::OBJECT_YOU_OWNER | flags::OBJECT_ANY_OWNER | flags::OBJECT_MODIFY | flags::OBJECT_COPY;
    const THEIRS: u32 = flags::OBJECT_ANY_OWNER;

    #[test]
    fn own_object_can_be_taken_copied_and_deleted() {
        let f = ObjectFacts::from_flags(MINE, 0, MINE, &[MINE, MINE], false, false);
        assert!(f.take && f.take_copy && f.delete && f.unlink);
        assert!(!f.buy && !f.pay && !f.touch);
    }

    #[test]
    fn someone_elses_object_cannot_be_deleted_unless_modifiable() {
        let f = ObjectFacts::from_flags(THEIRS, 0, THEIRS, &[THEIRS], false, true);
        assert!(!f.delete && !f.take && !f.take_copy && !f.unlink);
        assert!(f.buy);
        // modify rights given by its owner: deletable, not takeable without transfer
        let m = THEIRS | flags::OBJECT_MODIFY;
        let f = ObjectFacts::from_flags(m, 0, m, &[m], false, false);
        assert!(f.delete && !f.take);
        // public (no owner) objects can be deleted
        assert!(ObjectFacts::from_flags(0, 0, 0, &[0], false, false).delete);
    }

    #[test]
    fn delete_asks_first_for_locked_no_copy_or_others_objects() {
        let mine = MINE | flags::OBJECT_MOVE;
        assert_eq!(delete_warning(&[mine, mine]), None);
        let w = delete_warning(&[mine, mine & !flags::OBJECT_COPY]).unwrap_or_default();
        assert!(w.contains("pas copiable") && !w.contains("verrouillé") && !w.contains("appartient"));
        let w = delete_warning(&[THEIRS | flags::OBJECT_MODIFY]).unwrap_or_default();
        assert!(w.contains("verrouillé") && w.contains("pas copiable") && w.contains("appartient"));
    }

    #[test]
    fn touch_and_pay_follow_the_parent_and_sitting_blocks_take() {
        let f = ObjectFacts::from_flags(0, flags::HANDLE_TOUCH | flags::TAKES_MONEY, MINE, &[MINE], true, false);
        assert!(f.touch && f.pay);
        assert!(f.stand && !f.sit && !f.take);
    }
}
