//! Right-click menus with Firestorm's entries and order: the world (land,
//! objects, our attachments, other avatars, our avatar: menu_land.xml,
//! menu_object.xml, menu_attachment_self.xml, menu_avatar_other.xml,
//! menu_attachment_other.xml, menu_avatar_self.xml) and the avatar and
//! group names of lists (menu_people_nearby.xml, menu_fs_contacts_friends.xml,
//! menu_url_agent.xml, menu_people_groups.xml). ui/menu.rs draws the list;
//! context/pie.rs draws the radial world menu from their shared entries.
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

mod customize;
mod entries;
pub use customize::{Customization, Preferences};
mod avatar_textures;
pub use avatar_textures::AvatarTexturesUi;
mod object_profile;
pub use object_profile::ObjectProfileUi;
mod multi;
mod notecards;
mod pathfinding;
pub use pathfinding::PathfindingUi;
mod pie;

#[cfg(test)]
#[path = "context/sound_tests.rs"]
mod sound_tests;
#[cfg(test)]
mod toggle_tests;

pub use multi::avatars_menu;

/// Object update flags (llprimitive/object_flags.h).
pub mod flags {
    pub const OBJECT_MODIFY: u32 = 1 << 2;
    pub const OBJECT_COPY: u32 = 1 << 3;
    pub const OBJECT_ANY_OWNER: u32 = 1 << 4;
    pub const OBJECT_YOU_OWNER: u32 = 1 << 5;
    pub const SCRIPTED: u32 = 1 << 6;
    pub const HANDLE_TOUCH: u32 = 1 << 7;
    pub const OBJECT_MOVE: u32 = 1 << 8;
    pub const TAKES_MONEY: u32 = 1 << 9;
    /// FLAGS_AFFECTS_NAVMESH: flagObjectPermanent.
    pub const AFFECTS_NAVMESH: u32 = 1 << 12;
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
    /// (menu_attachment_other.xml adds "Object profile").
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
    /// Navigation in the radial menu, reset when opening a new menu.
    pub pie_path: Vec<usize>,
    /// Ignore the opening release and repeated egui passes in the same frame.
    pub pie_input_frame: Option<u64>,
}

/// What the object menu entries depend on (the on_enable functions of
/// llviewermenu.cpp), worked out every frame while the menu is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectFacts {
    /// Local meshes use the ordinary object menu and build selection, with
    /// only actions that can be carried out without a simulator identity.
    pub local_preview: bool,
    pub local_hud_scale: Option<f32>,
    pub saved_hud_scale: bool,
    pub hud_move_locked: bool,
    /// enable_object_edit on one of our attachments, body or HUD: the
    /// build tools can select it.
    pub edit: bool,
    pub attachment_item: Option<Uuid>,
    /// enable_detach: our attachment, not held by an RLVa remove lock.
    pub detach: bool,
    /// The prim the menu is about (nil in the menus of avatars and land).
    pub id: Uuid,
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
    /// Blocked (LLMuteList), for "Ignore" / "Stop ignoring".
    pub blocked: bool,
    /// Name from ObjectProperties (block list entry).
    pub name: String,
    /// The touch and sit texts of ObjectProperties: the labels of "Touch"
    /// and "Sit here" when set (enable_object_touch, enable_object_sit).
    pub touch_name: String,
    pub sit_name: String,
    /// Roots in the menu's selection: a right click on an object that is
    /// already selected keeps the selection (handleObjectSelection), and
    /// "Take" becomes "Take multiple" (Object.VisibleTakeMultiple).
    pub roots: usize,
    /// LLSelectMgr::enableLinkObjects.
    pub link: bool,
    /// object_is_wearable: "Wear", "Add" and the attachment points.
    pub wear: bool,
    /// EditableSelected: a prim of the selection we may modify (the
    /// script actions); EditableSelectedMono adds UpdateScriptTask.
    pub scripts: bool,
    pub mono: bool,
    /// Our attachment's inventory item is a favorite (None: not known).
    pub favorite: Option<bool>,
    /// Attachment.EnableDrop: ours, its item in the inventory, building
    /// allowed on the parcel.
    pub drop: bool,
    /// The skeleton of an animated object (LLAvatarEnableResetSkeleton).
    pub animesh: Option<Uuid>,
}

impl ObjectFacts {
    /// The rules on the update flags of the clicked prim, its root and all
    /// the prims of the linkset.
    pub fn from_flags(clicked: u32, parent: u32, root: u32, family: &[u32], sitting_on: bool, for_sale: bool) -> ObjectFacts {
        let has = |f: u32, bit: u32| f & bit != 0;
        let you_own = has(root, flags::OBJECT_YOU_OWNER);
        ObjectFacts {
            attachment_item: None,
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
    pub hover_height: bool,
    pub worn_shape: Option<Uuid>,
    pub object: ObjectFacts,
    /// Avatar: (complexity, shown as a silhouette).
    pub complexity: Option<(u32, bool)>,
    /// Rendering exception: 0 normal, 1 never fully, 2 always fully.
    pub render: u8,
    pub blocked: bool,
    /// Voice chat is muted for this resident (`LLMute::flagVoiceChat` only).
    pub voice_blocked: bool,
    /// Lindens cannot have their voice muted (`LLInspectAvatar`).
    pub can_block_voice: bool,
    /// The independent owner particle filter is enabled.
    pub particles_blocked: bool,
    /// Asset blacklist owner categories: 1 worn, 2 rezzed, 4 gesture.
    pub sound_blocks: u8,
    /// Our contact sets, for "Add to contact set" (getAllContactSets).
    pub contact_sets: Vec<String>,
    pub friend: bool,
    /// RLVa `@startim` forbids an IM with this resident (RlvActions::canStartIM).
    pub im_blocked: bool,
    /// EditLinkedParts.
    pub edit_linked: bool,
    /// ShowPropertyLines, shared with the World menu and terrain editor.
    pub property_lines: bool,
    /// The clicked emitter and distinct emitters whose particles crossed the click.
    pub particle_sources: Vec<ParticleMenuSource>,
    /// A particle over the sky: show only the Particles submenu.
    pub particle_only: bool,
    /// "Put on ▸ Attach" and "Attach HUD" (LLVOAvatarSelf::buildMenus).
    pub attach_points: Vec<AttachChoice>,
    /// Voice is enabled: "Call" works (LLAvatarActions::canCall).
    pub voice: bool,
    /// The avatar's voice row, when they are in our voice channel.
    pub voice_volume: Option<menu::VoiceVolume>,
    /// The avatar is in view and wears something: "Inspect" lists it.
    pub worn: bool,
    pub avatar_travel: bool,
    pub following: bool,
    /// Removable clothing types and occupied attachment points in our outfit.
    pub removable_clothing: Vec<u8>,
    pub removable_points: Vec<u8>,

    /// "Freeze" / "Eject" of the avatar, and whether the eject may ban.
    pub moderation: crate::world::land_moderation::Rights,
}

#[derive(Debug, Clone)]
pub struct ParticleMenuSource {
    pub object: Uuid,
    pub owner: Uuid,
    pub name: String,
    pub clicked: bool,
    pub object_blocked: bool,
    pub owner_blocked: bool,
}

/// One attachment point of "Put on ▸ Attach" or "Attach HUD".
#[derive(Debug, Clone, PartialEq)]
pub struct AttachChoice {
    pub id: u8,
    pub label: String,
    /// avatar_lad.xml group (the body part submenu; 9 is HUD) and place.
    pub group: u8,
    pub pie_slice: u8,
}

#[derive(Debug, Clone)]
pub enum CtxAction {
    /// Reopen this menu in the shared arrangement editor.
    CustomizeMenu(Box<ContextMenu>),
    SitAt(Vec3),
    SitStand,
    LookAtAvatar(Uuid),
    EditPbr(Uuid),
    ObjectProfile(Uuid),
    Pathfinding {
        id: Uuid,
        characters: bool,
    },
    ResetMeshLod(Uuid),
    ShowAvatarTextures(Uuid),
    TakeOffClothing(u8),
    DetachAt(u8),
    DetachAll,
    ResetHudScale(ObjKey),
    ForgetHudScale(ObjKey),
    SetHudMoveLock(ObjKey, bool),
    Detach(ObjKey),
    AttachmentInventory(Uuid),
    EditAttachment(ObjKey),
    /// "Touch" / "Edit" of a worn item in the inventory or the
    /// outfit lists, by inventory item (handle_attachment_touch /
    /// handle_attachment_edit).
    TouchWorn(Uuid),
    EditWorn(Uuid),
    /// "New physics" of the outfit gallery: created, worn and edited
    /// (LLAgentWearables::createWearable).
    NewPhysics,
    /// Avatar.Freeze / Avatar.Eject: asked first (LLAvatarActions::
    /// landFreeze / landEject).
    Freeze(Uuid),
    Eject(Uuid),
    Touch(u32),
    Sit {
        target: Uuid,
        offset: Vec3,
    },
    /// Camera on the clicked point; it follows the resident given, the one
    /// clicked or wearing the clicked attachment (Tools.LookAtSelection "zoom").
    Zoom(Vec3, Option<Uuid>),
    GoToAvatar(Uuid),
    FollowAvatar(Uuid),
    StopFollowing,
    /// About Land on the parcel at the clicked point (LLToolPie selects it).
    AboutLand(Vec3),
    /// Buy Land on the parcel at the clicked point (Land.Buy).
    BuyLand(Vec3),
    /// A pass for the parcel at the clicked point (Land.BuyPass).
    BuyPass(Vec3),
    SetPropertyLines(bool),
    StandUp,
    SitGround,
    ToggleFly,
    ResetCamera,
    Im(Uuid),
    /// Call a resident or a group (LLAvatarActions::startCall,
    /// LLGroupActions::startCall).
    Call(Uuid),
    /// The History window on the transcript of the nearby chat (None), of
    /// a conversation, a resident or a group.
    ShowHistory(Option<Uuid>),
    OfferTeleport(Uuid),
    /// A button of a teleport card in a conversation (accept or not).
    AnswerTeleport {
        invite: u64,
        accept: bool,
    },
    /// A button of a resident's inventory offer card (keep or reject).
    AnswerInventoryOffer {
        offer: u64,
        keep: bool,
    },
    Profile(Uuid),
    /// Rendering exception of an avatar (LLRenderMuteList).
    SetRender(Uuid, u8),
    /// "Reset skeleton" / "Reset skeleton and animations" of an avatar,
    /// ours included (Avatar.ResetSkeleton, Avatar.ResetSkeletonAndAnimations).
    ResetSkeleton {
        avatar: Uuid,
        animations: bool,
    },
    /// "Stop animations" of our avatar (Tools.StopAllAnimations:
    /// LLAgent::stopCurrentAnimations).
    StopAnimations,
    /// Me › Avatar health › "Stop avatar animations": the same, the scripts
    /// keeping their animation permissions (Tools.StopAllAnimations "stop").
    StopAnimationsKeepPermissions,
    /// Me › Avatar health › "Undeform avatar" (Tools.Undeform).
    Undeform,
    /// Me › Avatar health › "Refresh attachments" (Advanced.RefreshAttachments).
    RefreshAttachments,
    /// Block / unblock a resident (LLMuteList).
    ToggleBlock(Uuid),
    /// A resident's voice volume, 0..1 with 0.5 as sent (setUserVolume,
    /// LLSpeakerVolumeStorage).
    SetVoiceVolume(Uuid, f32),
    /// Block / unblock only a resident's voice (LLAvatarActions::
    /// toggleMuteVoice: LLMute::flagVoiceChat).
    ToggleVoiceMute(Uuid),
    /// Mute or unmute a resident (or everyone, None) in nearby voice for
    /// everyone, as its moderator (LLFloaterIMContainer::moderateVoice).
    ModerateVoice(Option<Uuid>, bool),
    /// Toggle the local owner particle filter, without creating a mute entry.
    ToggleOwnerParticleFilter(Uuid),
    /// One persistent owner category in the sound blacklist (ON blocks it).
    SetOwnerSoundBlock {
        owner: Uuid,
        flag: u8,
        blocked: bool,
    },
    /// "Invite to group" (LLAvatarActions::inviteToGroup): the groups where
    /// we may invite, then the chosen group's invitation window.
    InviteToGroup(Uuid),
    /// "Give my calling card" (LLAvatarGiveCard: OfferCallingCard).
    GiveCallingCard(Uuid),
    /// "Create landmark" where the agent is (About Land's Landmark).
    CreateLandmark,
    /// "Report Abuse" on a resident, an object or nothing (LLFloaterReporter).
    ReportAbuse(crate::report_abuse::ReportTarget),
    /// Block / unblock an object (LLMute::OBJECT).
    ToggleBlockObject {
        id: Uuid,
        name: String,
    },
    /// Edit the object in the build tools (handle_object_edit).
    Edit(Uuid),
    /// "Inspect": the window of the object's prims (Floater.Show "inspect").
    Inspect(Uuid),
    /// "Inspect" of an avatar: the window on everything it wears.
    InspectAvatar(Uuid),
    /// "Derender temporarily" / "Derender & blacklist" of an avatar or an
    /// object (Object.Derender / Object.DerenderPermanent, Avatar.Derender).
    Derender {
        id: Uuid,
        permanent: bool,
    },

    /// Firestorm Tools.ScriptInfo: read simulator script statistics via the bridge.
    ScriptInfo(Uuid),
    /// Extended getScriptInfo details, requested directly without a preference.
    ObjectInfo(Uuid),
    /// "Refresh textures" of an object or of something worn: every texture
    /// of its linkset is downloaded again (Object.TexRefresh).
    RefreshTextures(Uuid),
    /// "Refresh textures" of an avatar: its bakes (Avatar.TexRefresh).
    RefreshAvatarTextures(Uuid),
    /// "Force appearance update (rebake)" (Advanced.RebakeTextures).
    Rebake,
    /// Take / take a copy into the inventory (derez_objects).
    Take(Uuid),
    TakeCopy(Uuid),
    /// Delete to the trash (LLSelectMgr::selectDelete), asking first when
    /// it is locked, not copyable or not ours.
    Delete(Uuid),
    DeleteConfirmed(Uuid),
    /// Pay / buy / open the object (the click-action dialogs).
    Pay(Uuid),
    /// Pay a resident (Avatar.Pay, LLAvatarActions::pay).
    PayResident(Uuid),
    Buy(Uuid),
    Open(Uuid),
    /// Unlink the linkset (Tools.Unlink).
    Unlink(Uuid),
    /// Link the selected objects (Tools.Link).
    Link(Uuid),
    /// "Take as separate items" / "Take copies as separate items"
    /// (Object.TakeSeparate / TakeSeparateCopy): one item per root.
    TakeSeparate {
        id: Uuid,
        copy: bool,
    },
    /// "Return" (Object.Return): asked first (ReturnToOwner), then
    /// DeRezObject RETURN_TO_OWNER.
    Return(Uuid),
    /// "Put on" (Object.AttachToAvatar / AttachAddToAvatar): walk to the
    /// object, then ObjectAttach; point 0 is the object's own point.
    Wear {
        id: Uuid,
        point: u8,
        add: bool,
    },
    /// The script actions on the selection (Tools.SelectedScriptAction).
    Scripts {
        id: Uuid,
        action: crate::build::script_queue::QueueKind,
    },
    /// "Drop" one of our attachments (Attachment.Drop): asked first
    /// (AttachmentDrop), then ObjectDrop.
    Drop(ObjKey),
    /// Local particle-only filtering, independent of mute/derender.
    SetParticleBlock {
        id: Uuid,
        owner: bool,
        blocked: bool,
    },
    /// "Add to / Remove from favorites" of our attachment's inventory item
    /// (Object.SetFavorite).
    AttachmentFavorite(Uuid),
    /// "Edit linked parts" (Tools.EditLinkedParts).
    EditLinkedParts(bool),
    /// Land menu: build tools, create (Land.Build) or terraform (Land.Edit).
    Build,
    EditTerrain,
    /// Change our display name (LLFloaterDisplayName).
    DisplayName,
    HoverHeight,
    OpenAppearance {
        tab: usize,
        editing: bool,
    },
    /// People floater on a tab (0 nearby, 1 friends, 2 groups).
    OpenPeople(u8),
    /// Camera on an avatar of a list, following it (Avatar.ZoomIn).
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
    /// A `secondlife:///app/...` command clicked in a text (`slapp.rs`).
    AppLink(crate::slapp::AppLink),
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
    /// Several friends removed, after one confirmation (removeFriendsDialog).
    RemoveFriends(Vec<Uuid>),
    /// A text conference with these residents (startConference).
    Conference(Vec<Uuid>),
    /// A voice conference with these residents (startAdhocCall).
    ConferenceCall(Vec<Uuid>),
    /// A teleport offer to each of them (one prompt).
    OfferTeleports(Vec<Uuid>),
    /// Ask an avatar to teleport us to them (TeleportRequest prompt).
    RequestTeleport(Uuid),
    GroupChat(Uuid),
    /// Make a group active (ActivateGroup), pin it in the lists, leave it.
    ActivateGroup(Uuid),
    PinGroup(Uuid),
    LeaveGroup(Uuid),
    /// Block / unblock a group's chat (exoGroupMuteList).
    GroupChatBlocked(Uuid, bool),
    /// Contact sets and nicknames (LGGContactSets).
    ContactSet(crate::ui::contacts::SetEdit),
    /// "New contact set…" of an "Add to contact set" submenu: its name
    /// asked (AddNewContactSet), then these residents added to it.
    NewContactSet(Vec<Uuid>),
}

/// "Add to contact set": one entry per set, then "New contact set…"
/// (menu_fs_contacts_friends.xml, LLAvatarActions::addToContactSet: the
/// sets of FSFloaterAddToContactSet's list and its "+", AddNewContactSet).
pub fn contact_set_submenu(ui: &mut egui::Ui, p: &Palette, world: &World, ids: &[Uuid], label: &str) {
    let sets = world.contact_sets.set_names();
    menu::submenu(ui, p, "users-group", label, !ids.is_empty(), |ui| {
        if sets.is_empty() {
            menu::item_if(ui, p, "users-group", aurora_i18n::tr!("ui-contacts-no-contact-sets"), false);
        }
        for set in sets {
            if menu::item(ui, p, "users-group", &set) {
                request(
                    ui.ctx(),
                    CtxAction::ContactSet(crate::ui::contacts::SetEdit::Add { set, ids: ids.to_vec() }),
                );
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "plus", aurora_i18n::tr!("ui-context-new-contact-set")) {
            request(ui.ctx(), CtxAction::NewContactSet(ids.to_vec()));
        }
    });
}

/// egui temp data: actions chosen in menus this frame.
fn requests_id() -> egui::Id {
    egui::Id::unique("aurora_context_requests")
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
    world.avatar_map_position(*id)
}

/// egui temp data: row hovered last frame in the world menu.
const HOVERED: &str = "world_context_menu_hovered";

/// Draw the world menu; `menu` becomes `None` once it closed.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    menu: &mut Option<ContextMenu>,
    f: &Facts,
    use_pie: bool,
    prefs: &mut Preferences,
    customization: &mut Customization,
) -> bool {
    if menu.is_none() && !customization.active() {
        return false;
    }
    let saved_before = prefs.clone();
    if menu::dismiss_on_window_change(ctx) {
        *menu = None;
        ctx.data_mut(|d| d.remove::<Option<usize>>(egui::Id::unique(HOVERED)));
        customization.close(ctx, prefs);
        return *prefs != saved_before;
    }
    let Some(m) = menu.clone() else {
        customization.close(ctx, prefs);
        return *prefs != saved_before;
    };
    let mut open = true;
    let entries = entries::world_menu(&m.target, f, m.point);
    let hovered = if use_pie && !customization.force_list() {
        let voice = match m.target {
            Target::Avatar { id, own: false, .. } => f.voice_volume.map(|v| (id, v)),
            _ => None,
        };
        let (entries, customized) = customize::radial_entries(&m, f, &entries, prefs);
        let mut radial = m;
        let hovered = pie::show(ctx, p, &mut radial, &entries, voice, &mut open, customized);
        *menu = Some(radial);
        hovered
    } else {
        if let Some(m) = menu.as_mut() {
            m.pie_path.clear();
            m.pie_input_frame = None;
        }
        menu::popup_at(ctx, egui::Id::unique("world_context_menu"), m.pos, &mut open, p, |ui| {
            // This header is outside the arrangement catalogue: it cannot be
            // dragged, hidden or moved into an icon bar. Moderation is grouped
            // with the other scene actions instead of extending this header.
            if let Target::Avatar { id, own: false, .. } = m.target
                && let Some(voice) = f.voice_volume
            {
                voice_control(ui, p, id, voice);
                menu::separator(ui, p);
            }
            // The editor resets its UI's height. A child UI starts below the
            // fixed header, so that reset cannot rewind onto the voice row.
            ui.scope(|ui| customization.show(ui, p, &m, f, &entries, prefs));
        })
    };
    // UISndPieMenuSliceHighlight0..7 when an enabled row gets hovered (the
    // eight slices of Firestorm's default pie menu, PieMenu::draw)
    let before = ctx.data_mut(|d| d.get_temp::<Option<usize>>(egui::Id::unique(HOVERED)).flatten());
    if hovered != before {
        if let Some(i) = hovered.filter(|i| *i < 8) {
            super::sound_cues::request(ctx, crate::ui_sound::UiSound::PieMenuSlice(i as u8));
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(HOVERED), hovered));
    }
    if !open {
        *menu = None;
        customization.close(ctx, prefs);
        ctx.data_mut(|d| d.remove::<Option<usize>>(egui::Id::unique(HOVERED)));
    }
    *prefs != saved_before
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

/// A resident's voice row, only while they are in our voice channel and are
/// not us: LLInspectAvatar::updateVolumeSlider hides the slider and the mute
/// button otherwise, and a greyed row in every menu of everyone out of voice
/// would only push the entries down. The inspector held them; Aurora puts
/// them at the top of every menu of the resident.
pub fn voice_volume_of(world: &World, id: Uuid) -> Option<menu::VoiceVolume> {
    if id == world.agent_id || id.is_nil() || !world.calls.hears(&id) {
        return None;
    }
    let name = world.legacy_name(&id).unwrap_or_else(|| world.social.name_of(&id));
    Some(menu::VoiceVolume {
        volume: world.calls.volume(&id),
        muted: world.voice_blocked(&id),
        can_mute: !crate::world::mutes::is_linden(&name),
        moderated: world.nearby_voice_moderated(&id),
    })
}

/// The voice row and a separator under it.
fn voice_row(ui: &mut egui::Ui, p: &Palette, id: Uuid, voice: Option<menu::VoiceVolume>) {
    let Some(voice) = voice else {
        return;
    };
    voice_edit(ui, p, id, voice);
    menu::separator(ui, p);
}

/// The voice row alone, its changes handed to the app.
fn voice_edit(ui: &mut egui::Ui, p: &Palette, id: Uuid, voice: menu::VoiceVolume) {
    voice_control(ui, p, id, voice);
    voice_moderation(ui, p, id, voice);
}

fn voice_control(ui: &mut egui::Ui, p: &Palette, id: Uuid, voice: menu::VoiceVolume) {
    match menu::voice_volume(ui, p, voice) {
        Some(menu::VoiceEdit::Volume(v)) => request(ui.ctx(), CtxAction::SetVoiceVolume(id, v)),
        Some(menu::VoiceEdit::ToggleMute) => request(ui.ctx(), CtxAction::ToggleVoiceMute(id)),
        None => {}
    }
}

fn voice_moderation(ui: &mut egui::Ui, p: &Palette, id: Uuid, voice: menu::VoiceVolume) {
    // menu_conversation.xml's moderator entries (ModerateVoiceMuteSelected /
    // UnMuteSelected, ModerateVoiceMute / Unmute)
    if let Some(moderated) = voice.moderated {
        let (icon, label) = if moderated {
            ("microphone-filled", aurora_i18n::tr!("ui-context-voice-unmute-for-everyone"))
        } else {
            ("microphone-off", aurora_i18n::tr!("ui-context-voice-mute-for-everyone"))
        };
        if menu::item(ui, p, icon, label) {
            request(ui.ctx(), CtxAction::ModerateVoice(Some(id), !moderated));
        }
        if menu::item(ui, p, "microphone-off", aurora_i18n::tr!("ui-context-voice-mute-everyone")) {
            request(ui.ctx(), CtxAction::ModerateVoice(None, true));
        }
        if menu::item(ui, p, "microphone-filled", aurora_i18n::tr!("ui-context-voice-unmute-everyone")) {
            request(ui.ctx(), CtxAction::ModerateVoice(None, false));
        }
    }
}

/// Right-click menu of an avatar name in a list. Our own name gets only
/// the profile; a resident in our voice channel gets their voice volume
/// first.
pub fn avatar_list_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, kind: AvatarList) {
    avatar_menu_reporting(ui, p, world, id, kind, None);
}

/// The menu of the sender of a chat line: its "Report abuse" carries the
/// line (menu_avatar_icon.xml report_abuse, LLFloaterReporter::showFromChat).
pub fn chat_sender_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, line: &crate::world::ChatLine) {
    avatar_menu_reporting(ui, p, world, id, AvatarList::Name, Some(crate::report_abuse::chat_target(id, line)));
}

fn avatar_menu_reporting(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    id: Uuid,
    kind: AvatarList,
    report: Option<crate::report_abuse::ReportTarget>,
) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| request(&ctx, a);
    if id == world.agent_id {
        if menu::item(ui, p, "user-filled", aurora_i18n::tr!("common-view-profile")) {
            act(CtxAction::Profile(id));
        }
        return;
    }
    voice_row(ui, p, id, voice_volume_of(world, id));
    let friend = world.social.friends.iter().find(|f| f.id == id).cloned();
    let blocked = world.is_avatar_blocked(&id);
    let here = avatar_position(world, &id).is_some();
    let mappable = world.can_locate_avatar(id);
    let travel = !world.agent.is_sitting() && crate::autopilot::avatar_position(world, id).is_some();
    let following = world.following_avatar == Some(id);
    let online = friend.as_ref().is_none_or(|f| f.online);
    let name = world.social.name_of(&id);
    if !world.rlva_can_name(id) {
        // RLVa's restricted nearby/name menu exposes no profile, identity
        // clipboard or conversation-history routes (RlvActions::canShowName).
        if menu::item_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("common-offer-teleport"),
            online && world.rlva_can_offer_teleport(id),
        ) {
            act(CtxAction::OfferTeleport(id));
        }
        if menu::item_if(ui, p, "plane-arrival-filled", aurora_i18n::tr!("common-request-teleport"), online) {
            act(CtxAction::RequestTeleport(id));
        }
        if menu::item_if(ui, p, "zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), here) {
            act(CtxAction::ZoomAvatar(id));
        }
        let text = if blocked {
            aurora_i18n::tr!("common-unblock")
        } else {
            aurora_i18n::tr!("common-block")
        };
        if menu::item(ui, p, "ban", text) {
            act(CtxAction::ToggleBlock(id));
        }
        return;
    }
    let profile = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "user-filled", aurora_i18n::tr!("common-view-profile")) {
            act(CtxAction::Profile(id));
        }
    };
    // "can_send_im" of FSNameListAvatarMenu: RLVa @startim greys IM and the
    // call it would start (LLAvatarActions::startCall refuses it too)
    let can_im = world.rlva_can_start_im(id);
    let im = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "message-circle-filled", aurora_i18n::tr!("ui-context-send-im"), can_im) {
            act(CtxAction::Im(id));
        }
    };
    let offer_tp = |ui: &mut egui::Ui| {
        if menu::item_if(
            ui,
            p,
            "send-filled",
            aurora_i18n::tr!("common-offer-teleport"),
            online && world.rlva_can_offer_teleport(id),
        ) {
            act(CtxAction::OfferTeleport(id));
        }
    };
    let tp_to = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "location-filled", aurora_i18n::tr!("common-teleport-to"), mappable) {
            act(CtxAction::TeleportToAvatar(id));
        }
    };
    let zoom = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), here) {
            act(CtxAction::ZoomAvatar(id));
        }
    };
    let navigation = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "walk", aurora_i18n::tr!("ui-context-go-to-avatar"), travel) {
            act(CtxAction::GoToAvatar(id));
        }
        let label = if following {
            aurora_i18n::tr!("ui-context-stop-following")
        } else {
            aurora_i18n::tr!("common-follow")
        };
        if menu::item_if(ui, p, "current-location-filled", label, travel || following) {
            act(if following {
                CtxAction::StopFollowing
            } else {
                CtxAction::FollowAvatar(id)
            });
        }
    };
    let block = |ui: &mut egui::Ui| {
        if menu::check(ui, p, "ban", aurora_i18n::tr!("common-ignore"), blocked, true) {
            act(CtxAction::ToggleBlock(id));
        }
    };
    let copy = |ui: &mut egui::Ui| {
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-name")) {
            ui.ctx().copy_text(name.clone());
        }
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-url")) {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
        }
        if menu::item(ui, p, "at", aurora_i18n::tr!("common-copy-mention-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/mention"));
        }
    };
    let add_friend = |ui: &mut egui::Ui| {
        if friend.is_none() && menu::item(ui, p, "user-plus", aurora_i18n::tr!("common-become-friends")) {
            act(CtxAction::AddFriend(id));
        }
    };
    let remove_friend = |ui: &mut egui::Ui| {
        if friend.is_some() && menu::item(ui, p, "user-minus", aurora_i18n::tr!("common-remove-this-friend")) {
            act(CtxAction::RemoveFriend(id));
        }
    };
    let request_tp = |ui: &mut egui::Ui| {
        if menu::item_if(ui, p, "plane-arrival-filled", aurora_i18n::tr!("common-request-teleport"), online) {
            act(CtxAction::RequestTeleport(id));
        }
    };
    // the transcript of the conversation with this resident (enabled while
    // the account keeps transcripts; the window says when there is none)
    let history = |ui: &mut egui::Ui| {
        if menu::item_if(
            ui,
            p,
            "history",
            aurora_i18n::tr!("common-view-conversation-history"),
            world.chat_log.dir().is_some(),
        ) {
            act(CtxAction::ShowHistory(Some(id)));
        }
    };
    match kind {
        AvatarList::Nearby => {
            profile(ui);
            navigation(ui);
            im(ui);
            offer_tp(ui);
            request_tp(ui);
            tp_to(ui);
            if menu::item_if(
                ui,
                p,
                "phone-filled",
                aurora_i18n::tr!("common-voice-call"),
                world.calls.voice_enabled && can_im,
            ) {
                act(CtxAction::Call(id));
            }
            menu::separator(ui, p);
            history(ui);
            menu::separator(ui, p);
            add_friend(ui);
            remove_friend(ui);
            contact_set_submenu(ui, p, world, &[id], aurora_i18n::tr!("ui-context-add-to-contact-set"));
            // Avatar.InviteToGroup (can_invite: always)
            if menu::item(ui, p, "users-plus", aurora_i18n::tr!("ui-context-invite-to-group")) {
                act(CtxAction::InviteToGroup(id));
            }
            menu::separator(ui, p);
            zoom(ui);
            if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2"), mappable) {
                act(CtxAction::ShowOnMap(id));
            }
            menu::todo(ui, p, "share", aurora_i18n::tr!("common-share"));
            if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("common-pay")) {
                act(CtxAction::PayResident(id));
            }
            block(ui);
            // menu_fs_radar.xml / menu_mini_map.xml: Avatar.Derender and
            // Avatar.DerenderPermanent (LLAvatarActions::derender), always enabled
            for (icon, label, permanent) in [
                ("eye-off", aurora_i18n::tr!("ui-context-derender-temporarily"), false),
                ("eye-closed", aurora_i18n::tr!("ui-context-derender-blacklist"), true),
            ] {
                if menu::item(ui, p, icon, label) {
                    act(CtxAction::Derender { id, permanent });
                }
            }
        }
        AvatarList::Friend => {
            profile(ui);
            im(ui);
            history(ui);
            contact_set_submenu(ui, p, world, &[id], aurora_i18n::tr!("common-add-to-contact-set"));
            zoom(ui);
            tp_to(ui);
            if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2"), mappable) {
                act(CtxAction::ShowOnMap(id));
            }
            offer_tp(ui);
            request_tp(ui);
            if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("common-pay")) {
                act(CtxAction::PayResident(id));
            }
            navigation(ui);
            remove_friend(ui);
            menu::separator(ui, p);
            copy(ui);
            if let Some(f) = &friend {
                menu::separator(ui, p);
                // GlobalOnlineStatusToggle: the online status right we give
                let on = f.rights_given & 1 != 0;
                if menu::check(
                    ui,
                    p,
                    "eye-filled",
                    aurora_i18n::tr!("common-online-status-visible-to-this-friend"),
                    on,
                    true,
                ) {
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
            history(ui);
            add_friend(ui);
            contact_set_submenu(ui, p, world, &[id], aurora_i18n::tr!("ui-context-add-to-contact-set"));
            zoom(ui);
            tp_to(ui);
            offer_tp(ui);
            request_tp(ui);
            navigation(ui);
            remove_friend(ui);
            menu::separator(ui, p);
            if menu::item(ui, p, "flag-filled", aurora_i18n::tr!("ui-context-report-abuse")) {
                act(CtxAction::ReportAbuse(
                    report.clone().unwrap_or(crate::report_abuse::ReportTarget::Avatar(id)),
                ));
            }
            block(ui);
            menu::separator(ui, p);
            copy(ui);
        }
    }
}

/// Right-click menu of one of our groups (menu_people_groups.xml), plus
/// Aurora's "Block group chat" (exoGroupMuteList).
pub fn group_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) {
    let ctx = ui.ctx().clone();
    let act = |a: CtxAction| request(&ctx, a);
    if menu::item_if(
        ui,
        p,
        "circle-check-filled",
        aurora_i18n::tr!("common-enable"),
        world.groups.active != id,
    ) {
        act(CtxAction::ActivateGroup(id));
    }
    if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-view-info")) {
        super::group_profile::request_open(ui.ctx(), id);
    }
    if menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl-2")) {
        ui.ctx().copy_text(format!("secondlife:///app/group/{id}/about"));
    }
    // RLVa @startim with the group (LLGroupActions::startIM / startCall
    // refuse both): greyed here instead
    let can_im = world.rlva_can_start_im(id);
    if menu::item_if(ui, p, "message-circle-filled", aurora_i18n::tr!("common-chat"), can_im) {
        act(CtxAction::GroupChat(id));
    }
    // LLGroupActions::startCall (enabled with voice, isVoiceCallEnabled)
    if menu::item_if(
        ui,
        p,
        "phone-filled",
        aurora_i18n::tr!("common-voice-call"),
        world.calls.voice_enabled && can_im,
    ) {
        act(CtxAction::Call(id));
    }
    let pin = if world.groups.favorites.contains(&id) {
        aurora_i18n::tr!("common-unpin-group")
    } else {
        aurora_i18n::tr!("common-pin-group")
    };
    if menu::item(ui, p, "pin-filled", pin) {
        act(CtxAction::PinGroup(id));
    }
    let blocked = world.mutes.group_chat_muted(&id);
    if menu::check(ui, p, "message-off", aurora_i18n::tr!("ui-context-block-group-chat"), blocked, true) {
        act(CtxAction::GroupChatBlocked(id, !blocked));
    }
    menu::separator(ui, p);
    if menu::item(ui, p, "logout", aurora_i18n::tr!("common-quit")) {
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
        lines.push(aurora_i18n::tr!("ui-context-at-least-one-object-is-locked"));
    }
    if no_copy {
        lines.push(aurora_i18n::tr!("ui-context-at-least-one-object-is-no-copy"));
    }
    if not_ours {
        lines.push(aurora_i18n::tr!("ui-context-at-least-one-object-is-not-yours"));
    }
    (!lines.is_empty()).then(|| {
        aurora_i18n::tr!(
            "ui-context-are-you-sure-you-want-to-delete-these-objects",
            value0 = lines
                .join(
                    "
"
                )
                .to_string()
        )
    })
}

/// The delete confirmation; Some(true) to delete, Some(false) to cancel.
pub fn delete_confirm(ctx: &egui::Context, p: &Palette, text: &str) -> Option<bool> {
    use crate::ui::widgets::{ActionKind, action_button};
    use egui::RichText;
    let mut answer = None;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique("object_delete_confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-context-delete-object"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(12.5).color(p.muted));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = egui::Vec2::new(0.0, 26.0);
            let delete = aurora_i18n::tr!("common-delete");
            if action_button(ui, p, "trash-filled", delete, ActionKind::Danger, size).clicked() {
                answer = Some(true);
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// A question of the object menus with its "ignore" box (the okcancelignore
/// template: ReturnToOwner, AttachmentDrop, ConfirmScriptModify).
/// Some(true) goes on, Some(false) cancels; the caller saves `dont_ask`
/// once the answer is yes, like `teleport_confirm`.
pub struct Question<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub text: &'a str,
    pub yes: &'a str,
    pub no: &'a str,
}

pub fn ask(ctx: &egui::Context, p: &Palette, q: &Question, dont_ask: &mut bool) -> Option<bool> {
    use egui::RichText;
    let mut answer = None;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique(q.id)).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(q.title).size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(q.text).size(12.5).color(p.muted));
        ui.add_space(8.0);
        super::widgets::checkbox(
            ui,
            p,
            dont_ask,
            RichText::new(aurora_i18n::tr!("ui-context-do-not-show-again"))
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if super::widgets::default_button(super::widgets::flat_button(ui, p, q.yes)).clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, q.no).clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// One button of `choose`: icon, label and face.
pub type Choice<'a> = (&'a str, &'a str, crate::ui::widgets::ActionKind);

/// A question answered by one of its own buttons, then "Cancel" (the
/// yesnocancelbuttons / okcancelbuttons templates: FreezeAvatarFullname,
/// EjectAvatarFullname). Some(Some(i)) for `choices[i]`, Some(None) on
/// "Cancel" or Escape.
pub fn choose(ctx: &egui::Context, p: &Palette, id: &str, title: &str, lines: &[&str], choices: &[Choice]) -> Option<Option<usize>> {
    use crate::ui::widgets::{ActionKind, action_button};
    use egui::RichText;
    let mut answer = None;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
        for line in lines {
            ui.add_space(6.0);
            ui.label(RichText::new(*line).size(12.5).color(p.muted));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = egui::Vec2::new(0.0, 26.0);
            for (i, &(icon, label, kind)) in choices.iter().enumerate() {
                if action_button(ui, p, icon, label, kind, size).clicked() {
                    answer = Some(Some(i));
                }
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(None);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(None);
    }
    answer
}

/// TeleportViaSLAPP: the question of a teleport asked by a place link.
pub fn place_teleport_question(region: &str) -> String {
    aurora_i18n::tr!("ui-context-do-you-really-want-to-teleport-to", region = region.to_string())
}

/// A teleport confirmation (TeleportFromLandmark, TeleportToHistoryEntry,
/// TeleportViaSLAPP: the "okcancelignore" template of notifications.xml).
/// Some(true) teleports, Some(false) cancels; `dont_ask` is the "Do not
/// show again" box, which the caller saves as `Settings::confirm_teleports`
/// once the teleport is accepted. Firestorm keeps the box whatever the
/// answer (LLNotification::respond, llnotifications.cpp): here "Cancel"
/// leaves the setting alone, a refused teleport silences nothing.
pub fn teleport_confirm(ctx: &egui::Context, p: &Palette, id: &str, text: &str, dont_ask: &mut bool) -> Option<bool> {
    use egui::RichText;
    let mut answer = None;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("common-teleport-2"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(12.5).color(p.muted));
        ui.add_space(8.0);
        super::widgets::checkbox(
            ui,
            p,
            dont_ask,
            RichText::new(aurora_i18n::tr!("ui-context-do-not-show-again"))
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if super::widgets::default_button(super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-teleport-3"))).clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
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
/// `dont_warn` is the "Do not warn me again" box. A dangerous link
/// (`danger`: the reason, from `link_trust`) gets the red version, without
/// that box: it always warns.
pub fn external_link_confirm(ctx: &egui::Context, p: &Palette, url: &str, dont_warn: &mut bool, danger: Option<&str>) -> Option<bool> {
    use egui::RichText;
    let mut answer = None;
    let modal = crate::ui::widgets::Modal::new(egui::Id::unique("external_link_confirm")).show(ctx, |ui| {
        ui.set_width(380.0);
        let (icon, col, title) = match danger {
            Some(_) => ("circle-x-filled", p.danger, aurora_i18n::tr!("ui-context-dangerous-link")),
            None => (
                "alert-triangle-filled",
                p.amber,
                aurora_i18n::tr!("ui-context-think-before-clicking"),
            ),
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
                aurora_i18n::tr!("ui-context-aurora-recommends-against-opening-this-link-if-it-asks")
            }
            None => {
                aurora_i18n::tr!("ui-context-this-link-leads-to-a-site-that-is-not")
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
            super::widgets::checkbox(
                ui,
                p,
                dont_warn,
                RichText::new(aurora_i18n::tr!("ui-context-do-not-warn-me-again"))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
        }
        // the action, then "Cancel", like the other confirmations
        ui.horizontal(|ui| {
            let visit = if danger.is_some() {
                aurora_i18n::tr!("ui-context-visit-anyway")
            } else {
                aurora_i18n::tr!("ui-context-visit-link")
            };
            if super::widgets::default_button(super::widgets::flat_button(ui, p, visit)).clicked() {
                answer = Some(true);
            }
            if super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
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

    #[test]
    fn remote_friend_menu_location_clicks_follow_live_received_rights() {
        use crate::ui::widgets::elide_tests::{Frames, painted};
        use crate::world::social::{Friend, rights};
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let id = Uuid::from_u128(7);
        world.social.friends.push(Friend {
            id,
            online: true,
            rights_given: rights::MAP_LOCATION,
            rights_has: 0,
        });
        let p = crate::theme::Theme::default().palette();
        for kind in [AvatarList::Friend, AvatarList::Name, AvatarList::Nearby] {
            for (key, map) in [
                ("common-teleport-to", false),
                ("common-show-on-map-2", true),
                ("common-zoom-2", false),
            ] {
                if map && kind == AvatarList::Name {
                    continue;
                }
                let mut frames = Frames::new();
                let mut point = egui::Pos2::ZERO;
                for (received, online) in [(0, true), (rights::MAP_LOCATION, true), (0, true), (rights::MAP_LOCATION, false)] {
                    world.social.friends[0].rights_has = received;
                    world.social.friends[0].online = online;
                    for _ in 0..2 {
                        let texts = frames.run(Vec::new(), |ui| avatar_list_menu(ui, &p, &world, id, kind));
                        point = painted(&texts, aurora_i18n::text(key)).rect.center();
                    }
                    for pressed in [true, false] {
                        frames.run(
                            vec![
                                egui::Event::PointerMoved(point),
                                egui::Event::PointerButton {
                                    pos: point,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                            |ui| avatar_list_menu(ui, &p, &world, id, kind),
                        );
                    }
                    let actions = take_requests(&frames.ctx);
                    let enabled = received != 0 && online && key != "common-zoom-2";
                    assert_eq!(actions.len(), usize::from(enabled), "{kind:?}, {key}, {received}, {online}");
                    if enabled {
                        assert!(
                            matches!(actions[0], CtxAction::ShowOnMap(i) if map && i == id)
                                || matches!(actions[0], CtxAction::TeleportToAvatar(i) if !map && i == id)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rlva_hidden_identity_menu_keeps_zoom_and_block_without_profile_or_clipboard_routes() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let id = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .map(|(_, o)| o.full_id)
            .unwrap();
        world.rlva.configure(true);
        world.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Menu fixture".into(),
            source_id: Uuid::from_u128(1),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        let theme = crate::theme::Theme::default();
        let p = theme.palette();
        let ctx = egui::Context::default();
        theme.apply(&ctx, 1.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                avatar_list_menu(ui, &p, &world, id, AvatarList::Name);
            },
        );
        // This headless fixture inspects shapes without a texture consumer.
        output.textures_delta.clear();
        fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        collect(s, texts);
                    }
                }
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect(&shape.shape, &mut texts);
        }
        assert!(texts.iter().any(|t| t.contains(aurora_i18n::tr!("common-zoom-2"))));
        assert!(
            texts
                .iter()
                .any(|t| t.contains(aurora_i18n::tr!("common-block")) || t.contains(aurora_i18n::tr!("common-unblock")))
        );
        for label in [
            aurora_i18n::tr!("common-view-profile"),
            aurora_i18n::tr!("common-copy-name"),
            aurora_i18n::tr!("common-copy-url"),
            aurora_i18n::tr!("common-copy-mention-uri"),
            aurora_i18n::tr!("common-view-conversation-history"),
        ] {
            assert!(!texts.iter().any(|t| t.contains(label)), "unexpected identity route: {label}");
        }
    }

    #[test]
    fn modifier_click_opens_the_editing_of_an_attachment_the_tools_can_select() {
        fn label_pos(shape: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("common-edit") => Some(t.pos + egui::vec2(4.0, 4.0)),
                egui::epaint::Shape::Vec(v) => v.iter().find_map(label_pos),
                _ => None,
            }
        }
        for edit in [true, false] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let key = ObjKey { region: 1, local_id: 42 };
            let facts = Facts {
                object: ObjectFacts {
                    edit,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut current = Some(ContextMenu {
                pos: egui::pos2(200.0, 80.0),
                point: Vec3::ZERO,
                target: Target::Attachment { key },
                pie_path: Vec::new(),
                pie_input_frame: None,
            });
            let mut frame = |events: Vec<egui::Event>| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        show(
                            ui.ctx(),
                            &theme.palette(),
                            &mut current,
                            &facts,
                            false,
                            &mut Default::default(),
                            &mut Default::default(),
                        );
                    },
                );
                output.textures_delta.clear();
                output
            };
            for _ in 0..3 {
                frame(Vec::new());
            }
            let output = frame(Vec::new());
            let pos = output.shapes.iter().find_map(|s| label_pos(&s.shape)).expect("Modifier menu row");
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            let requests = take_requests(&ctx);
            if edit {
                assert!(matches!(&requests[..], [CtxAction::EditAttachment(k)] if *k == key));
            } else {
                assert!(requests.is_empty());
            }
        }
    }

    fn voice_world() -> (World, Uuid) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let tess = Uuid::from_u128(2);
        world.social.names.insert(tess, "Tess Touch".into());
        (world, tess)
    }

    #[test]
    fn only_a_resident_in_our_voice_channel_gets_the_voice_row() {
        use crate::world::mutes::{MuteType, flag};
        let (mut world, tess) = voice_world();
        assert_eq!(voice_volume_of(&world, tess), None, "out of voice: no row");
        world.calls.voice_peers.extend([tess, world.agent_id]);
        assert_eq!(voice_volume_of(&world, world.agent_id), None, "never ourselves");
        assert_eq!(
            voice_volume_of(&world, tess),
            Some(menu::VoiceVolume {
                volume: 0.5,
                muted: false,
                can_mute: true,
                moderated: None,
            })
        );
        world.calls.set_volume(tess, 0.8);
        assert_eq!(voice_volume_of(&world, tess).map(|v| v.volume), Some(0.8));
        // the voice flag of the block list mutes them; a full block too
        world
            .block_flags(tess, "Tess Touch", MuteType::Agent, flag::VOICE_CHAT)
            .expect("voice block");
        assert_eq!(voice_volume_of(&world, tess).map(|v| v.muted), Some(true));
        // a Linden's voice cannot be muted from the row
        let oz = Uuid::from_u128(3);
        world.social.names.insert(oz, "Oz Linden".into());
        world.calls.voice_peers.insert(oz);
        assert_eq!(voice_volume_of(&world, oz).map(|v| v.can_mute), Some(false));
    }

    #[test]
    fn the_voice_percentage_puts_the_volume_as_sent_at_100() {
        assert_eq!(menu::volume_percent(0.0), 0);
        assert_eq!(menu::volume_percent(0.5), 100);
        assert_eq!(menu::volume_percent(0.8), 160);
        assert_eq!(menu::volume_percent(1.0), 200);
        assert_eq!(menu::volume_percent(3.0), 200);
    }

    /// The rectangle of a text painted this frame.
    fn text_rect(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Rect> {
        fn find(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        shapes.iter().find_map(|s| find(&s.shape, text))
    }

    #[test]
    fn the_voice_row_tops_the_menu_and_its_clicks_keep_a_more_menu_open() {
        let (mut world, tess) = voice_world();
        world.calls.voice_peers.insert(tess);
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let popup = std::cell::Cell::new(egui::Id::NULL);
        let mut frame = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    // a "more" button's menu, closed by any click like a stock menu button
                    let resp = ui.allocate_response(egui::vec2(40.0, 20.0), egui::Sense::click());
                    popup.set(egui::Popup::default_response_id(&resp));
                    menu::dropdown(&resp, &p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        avatar_list_menu(ui, &p, &world, tess, AvatarList::Nearby)
                    });
                },
            );
            output.textures_delta.clear();
            output
        };
        frame(Vec::new());
        egui::Popup::open_id(&ctx, popup.get());
        for _ in 0..3 {
            frame(Vec::new());
        }
        let output = frame(Vec::new());
        let percent = text_rect(&output.shapes, "100 %").expect("voice row percentage");
        let profile = text_rect(&output.shapes, aurora_i18n::tr!("common-view-profile")).expect("profile row");
        assert!(percent.bottom() <= profile.top(), "the voice row comes first");
        let click = |frame: &mut dyn FnMut(Vec<egui::Event>) -> egui::FullOutput, pos: egui::Pos2| {
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        };
        // a click near the start of the slider (where the labels start): a lower volume
        click(&mut frame, egui::pos2(profile.left() + 12.0, percent.center().y));
        let requests = take_requests(&ctx);
        assert!(
            !requests.is_empty()
                && requests
                    .iter()
                    .all(|r| matches!(r, CtxAction::SetVoiceVolume(id, v) if *id == tess && *v < 0.5)),
            "{requests:?}"
        );
        assert!(egui::Popup::is_id_open(&ctx, popup.get()), "the slider leaves the menu open");
        // the mute button, right of the percentage
        click(&mut frame, egui::pos2(percent.right() + 16.0, percent.center().y));
        let requests = take_requests(&ctx);
        assert!(
            matches!(&requests[..], [CtxAction::ToggleVoiceMute(id)] if *id == tess),
            "{requests:?}"
        );
        assert!(egui::Popup::is_id_open(&ctx, popup.get()), "the mute button leaves the menu open");
        // an entry still closes it
        click(&mut frame, profile.center());
        assert!(matches!(&take_requests(&ctx)[..], [CtxAction::Profile(id)] if *id == tess));
        frame(Vec::new());
        assert!(!egui::Popup::is_id_open(&ctx, popup.get()));
    }

    #[test]
    fn scene_voice_header_stays_above_customizable_rows() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let id = Uuid::from_u128(123);
        let facts = Facts {
            voice_volume: Some(menu::VoiceVolume {
                volume: 0.5,
                muted: false,
                can_mute: true,
                moderated: None,
            }),
            ..Default::default()
        };
        let mut prefs = Preferences::default();
        let mut customization = Customization::default();
        for editing in [false, true] {
            let mut current = Some(ContextMenu {
                target: Target::Avatar {
                    id,
                    own: false,
                    attachment: Some(Uuid::from_u128(456)),
                },
                pos: egui::pos2(80.0, 80.0),
                point: Vec3::ZERO,
                pie_path: Vec::new(),
                pie_input_frame: None,
            });
            if editing {
                customization.edit(&ctx, current.as_ref().expect("menu"), &facts, &mut prefs);
            }
            let mut output = egui::FullOutput::default();
            for _ in 0..4 {
                // This layout/input test has no renderer to apply texture deltas.
                output = crate::ui::login::test_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui.ctx(), &p, &mut current, &facts, false, &mut prefs, &mut customization);
                    },
                ));
            }
            let volume = text_rect(&output.shapes, "100 %").expect("volume");
            let profile = text_rect(&output.shapes, aurora_i18n::tr!("common-view-profile")).expect("profile");
            assert!(
                volume.bottom() < profile.top(),
                "editing={editing}: {volume:?} overlaps {profile:?}"
            );
            let saved = prefs.clone();
            let slider = egui::pos2(profile.left() + 12.0, volume.center().y);
            let mute = egui::pos2(volume.right() + 16.0, volume.center().y);
            for (at, is_mute) in [(slider, false), (mute, true)] {
                take_requests(&ctx);
                for pressed in [true, false] {
                    let _ = crate::ui::login::test_output(ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                            events: vec![
                                egui::Event::PointerMoved(at),
                                egui::Event::PointerButton {
                                    pos: at,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                            ..Default::default()
                        },
                        |ui| {
                            show(ui.ctx(), &p, &mut current, &facts, false, &mut prefs, &mut customization);
                        },
                    ));
                }
                let actions = take_requests(&ctx);
                if is_mute {
                    assert!(
                        matches!(&actions[..], [CtxAction::ToggleVoiceMute(who)] if *who == id),
                        "{actions:?}"
                    );
                } else {
                    assert!(
                        !actions.is_empty()
                            && actions
                                .iter()
                                .all(|a| matches!(a, CtxAction::SetVoiceVolume(who, v) if *who == id && *v < 0.5)),
                        "{actions:?}"
                    );
                }
                assert!(current.is_some(), "voice controls leave the scene menu open");
                assert_eq!(prefs, saved, "fixed voice controls never customize the menu");
            }
            customization.close(&ctx, &mut prefs);
        }
    }

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
        assert!(
            w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-no-copy"))
                && !w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-locked"))
                && !w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-not-yours"))
        );
        let w = delete_warning(&[THEIRS | flags::OBJECT_MODIFY]).unwrap_or_default();
        assert!(
            w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-locked"))
                && w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-no-copy"))
                && w.contains(aurora_i18n::tr!("ui-context-at-least-one-object-is-not-yours"))
        );
    }

    #[test]
    fn touch_and_pay_follow_the_parent_and_sitting_blocks_take() {
        let f = ObjectFacts::from_flags(0, flags::HANDLE_TOUCH | flags::TAKES_MONEY, MINE, &[MINE], true, false);
        assert!(f.touch && f.pay);
        assert!(f.stand && !f.sit && !f.take);
    }
}
