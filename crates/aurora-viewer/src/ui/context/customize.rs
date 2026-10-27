//! Scene menus reuse the inventory arrangement editor and its bounded resolver.
//! Only presentation is saved. Commands are rebuilt from the current pick and
//! permissions, so a saved menu can never retain a previous object's identity.
use super::{ContextMenu, CtxAction, Facts, Target, entries};
use crate::ui::inventory::{context_customize, menu_layout as layout};
use std::collections::BTreeMap;

pub use layout::Preferences;

#[derive(Default)]
pub struct Customization {
    editor: context_customize::State,
    scope: Option<String>,
    list: bool,
    pub(crate) notecards: super::notecards::State,
}

pub(super) fn scope(target: &Target, facts: &Facts) -> &'static str {
    match target {
        Target::Ground => "scene:land",
        Target::Object { .. } => "scene:object",
        Target::Attachment { .. } if facts.object.local_hud_scale.is_some() => "scene:hud",
        Target::Attachment { .. } => "scene:attachment",
        Target::Avatar { own: true, .. } => "scene:self",
        Target::Avatar { attachment: Some(_), .. } => "scene:other-attachment",
        Target::Avatar { .. } => "scene:avatar",
    }
}

fn profile<'a>(prefs: &'a mut Preferences, scope: &str) -> &'a mut layout::Profile {
    prefs.profiles.entry(scope.into()).or_insert_with(|| layout::Profile {
        layout: layout::Layout::default(),
        open: BTreeMap::new(),
        locked: true,
    })
}

impl Customization {
    pub(super) fn active(&self) -> bool {
        self.scope.is_some()
    }
    pub(crate) fn edit(&mut self, ctx: &egui::Context, menu: &ContextMenu, facts: &Facts, prefs: &mut Preferences) {
        let scope = scope(&menu.target, facts);
        self.close(ctx, prefs);
        self.scope = Some(scope.into());
        self.list = true;
        profile(prefs, scope).locked = false;
    }

    pub(crate) fn close(&mut self, ctx: &egui::Context, prefs: &mut Preferences) {
        if let Some(scope) = self.scope.take() {
            self.editor.relock(ctx, profile(prefs, &scope));
        }
        self.editor = Default::default();
        self.list = false;
    }

    pub(super) fn force_list(&self) -> bool {
        self.list
    }

    pub(crate) fn demo_notecard(&mut self, load: bool) {
        use crate::ui::inventory::context_notecards::Mode;
        self.notecards.demo(if load { Mode::Load } else { Mode::Export });
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &crate::theme::Palette,
        menu: &ContextMenu,
        facts: &Facts,
        entries: &[entries::Entry],
        prefs: &mut Preferences,
    ) {
        let scope = scope(&menu.target, facts);
        let item = match menu.target {
            Target::Object { full_id, .. } => full_id,
            Target::Attachment { .. } => facts.object.id,
            Target::Avatar { id, attachment, .. } => attachment.unwrap_or(id),
            Target::Ground => uuid::Uuid::nil(),
        };
        self.scope = Some(scope.into());
        let catalog = Catalog::new(entries);
        self.editor.help = catalog
            .originals
            .iter()
            .filter(|(_, entry)| matches!(entry.kind, entries::Kind::Unavailable))
            .map(|(id, _)| (id.clone(), crate::ui::menu::not_yet().into()))
            .collect();
        let root = ui.layer_id().id;
        self.editor.prepare(ui.ctx(), item, root, scope, prefs);
        let profile = profile(prefs, scope);
        if let Some(index) = context_customize::show(ui, p, item, scope, &catalog.entries, profile, &mut self.editor)
            && profile.locked
            && let Some(
                entry @ entries::Entry {
                    enabled: true,
                    kind: entries::Kind::Action(action),
                    ..
                },
            ) = catalog.commands.get(index)
        {
            super::request(ui.ctx(), action.clone());
            if !entry.keeps_open() {
                ui.close();
            }
        }
        if let Some(mode) = self.editor.notecard.take() {
            self.notecards.open(mode);
        }
    }
}

pub(super) struct Catalog {
    pub(super) entries: Vec<layout::Entry>,
    commands: Vec<entries::Entry>,
    originals: BTreeMap<String, entries::Entry>,
}

impl Catalog {
    pub(super) fn new(entries: &[entries::Entry]) -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            commands: Vec::new(),
            originals: BTreeMap::new(),
        };
        catalog.flatten(entries, layout::TEXT, "", true);
        catalog.entries.push(layout::Entry {
            id: layout::LOCK.into(),
            label: String::new(),
            icon: "lock-filled".into(),
            enabled: true,
            checked: None,
            kind: layout::Kind::Lock,
            zone: "bottom".into(),
            slot: Some(7),
        });
        catalog
    }

    fn flatten(&mut self, entries: &[entries::Entry], zone: &str, parent: &str, enabled: bool) {
        let mut occurrences = BTreeMap::<String, usize>::new();
        for entry in entries {
            let key = match &entry.kind {
                entries::Kind::Action(action) => action_key(action),
                entries::Kind::Submenu(_) => format!("group:{}", entry.icon),
                entries::Kind::Separator => "separator".into(),
                _ => format!("row:{}", entry.icon),
            };
            let occurrence = occurrences.entry(key.clone()).or_default();
            let id = format!("{parent}/{key}:{occurrence}");
            *occurrence += 1;
            let kind = match &entry.kind {
                entries::Kind::Submenu(children) => {
                    self.flatten(children, &layout::children(&id), &id, enabled && entry.enabled);
                    layout::Kind::Group {
                        direction: layout::Direction::Horizontal,
                        open: false,
                    }
                }
                entries::Kind::Separator => layout::Kind::Separator,
                _ => {
                    let index = self.commands.len();
                    self.commands.push(entry.clone());
                    layout::Kind::Action(index)
                }
            };
            self.originals.insert(id.clone(), entry.clone());
            self.entries.push(layout::Entry {
                id,
                label: entry.label.clone(),
                icon: entry.icon.into(),
                enabled: enabled && entry.enabled,
                checked: entry.checked,
                kind,
                zone: zone.into(),
                slot: None,
            });
        }
    }

    fn radial(&self, resolved: &layout::Resolved, zone: &str, depth: usize) -> Vec<entries::Entry> {
        if depth > 24 {
            return Vec::new();
        }
        resolved
            .get(zone)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let mut original = self.originals.get(&entry.id).cloned().unwrap_or_else(|| entries::Entry {
                    icon: "folder-filled",
                    label: entry.label.clone(),
                    enabled: true,
                    checked: None,
                    kind: entries::Kind::Info,
                });
                original.label = entry.label.clone();
                original.enabled = entry.enabled;
                original.kind = match &entry.kind {
                    layout::Kind::Lock => return None,
                    layout::Kind::Group { .. } => entries::Kind::Submenu(self.radial(resolved, &layout::children(&entry.id), depth + 1)),
                    layout::Kind::IconBar => {
                        original.icon = "grip-vertical";
                        original.label = aurora_i18n::tr!("inventory-menu-icon-bar").into();
                        entries::Kind::Submenu(self.radial(resolved, &layout::icon_zone(&entry.id), depth + 1))
                    }
                    layout::Kind::Separator => entries::Kind::Separator,
                    layout::Kind::Action(index) => self.commands.get(*index)?.kind.clone(),
                };
                if matches!(&original.kind, entries::Kind::Submenu(children) if children.is_empty()) {
                    return None;
                }
                Some(original)
            })
            .collect()
    }
}

pub(super) fn radial_entries(
    menu: &ContextMenu,
    facts: &Facts,
    entries: &[entries::Entry],
    prefs: &Preferences,
) -> (Vec<entries::Entry>, bool) {
    let profile = prefs.profiles.get(scope(&menu.target, facts));
    let customized = profile.is_some_and(|p| p.layout != layout::Layout::default());
    let mut result = if let Some(profile) = profile.filter(|_| customized) {
        let catalog = Catalog::new(entries);
        let mut resolved = profile.layout.resolve(&catalog.entries);
        for (zone, rows) in &mut resolved {
            if layout::bar(zone) {
                let slots = profile.layout.icon_slots(zone, rows);
                rows.sort_by_key(|row| slots.iter().position(|id| id.as_ref() == Some(&row.id)).unwrap_or(usize::MAX));
            }
        }
        layout::hide_empty_groups(&mut resolved);
        catalog.radial(&resolved, layout::TEXT, 0)
    } else {
        entries.to_vec()
    };
    result.push(entries::Entry {
        icon: "adjustments-filled",
        label: aurora_i18n::tr!("scene-menu-customize").into(),
        enabled: true,
        checked: None,
        kind: entries::Kind::Action(CtxAction::CustomizeMenu(Box::new(menu.clone()))),
    });
    (result, customized)
}

/// Variant names are explicit presentation keys, independent of locale,
/// object UUIDs, coordinates and changing toggle state.
fn action_key(action: &CtxAction) -> String {
    let base = match action {
        CtxAction::CustomizeMenu(..) => "CustomizeMenu",
        CtxAction::SitAt(..) => "SitAt",
        CtxAction::SitStand => "SitStand",
        CtxAction::Freeze(..) => "Freeze",
        CtxAction::Eject(..) => "Eject",
        CtxAction::LookAtAvatar(..) => "LookAtAvatar",
        CtxAction::EditPbr(..) => "EditPbr",
        CtxAction::ObjectProfile(..) => "ObjectProfile",
        CtxAction::Pathfinding { characters: false, .. } => "RegionObjects",
        CtxAction::Pathfinding { characters: true, .. } => "PathfindingCharacters",
        CtxAction::ResetMeshLod(..) => "ResetMeshLod",
        CtxAction::ShowAvatarTextures(..) => "ShowAvatarTextures",
        CtxAction::TakeOffClothing(..) => "TakeOffClothing",
        CtxAction::DetachAt(..) => "DetachAt",
        CtxAction::DetachAll => "DetachAll",
        CtxAction::ResetHudScale(..) => "ResetHudScale",
        CtxAction::ForgetHudScale(..) => "ForgetHudScale",
        CtxAction::SetHudMoveLock(..) => "SetHudMoveLock",
        CtxAction::Detach(..) => "Detach",
        CtxAction::AttachmentInventory(..) => "AttachmentInventory",
        CtxAction::EditAttachment(..) => "EditAttachment",
        CtxAction::TouchWorn(..) => "TouchWorn",
        CtxAction::EditWorn(..) => "EditWorn",
        CtxAction::NewPhysics => "NewPhysics",
        CtxAction::Touch(..) => "Touch",
        CtxAction::Sit { .. } => "Sit",
        CtxAction::Zoom(..) => "Zoom",
        CtxAction::GoToAvatar(..) => "GoToAvatar",
        CtxAction::FollowAvatar(..) => "FollowAvatar",
        CtxAction::StopFollowing => "FollowAvatar",
        CtxAction::AboutLand(..) => "AboutLand",
        CtxAction::BuyLand(..) => "BuyLand",
        CtxAction::BuyPass(..) => "BuyPass",
        CtxAction::SetPropertyLines(..) => "SetPropertyLines",
        CtxAction::StandUp => "StandUp",
        CtxAction::SitGround => "SitGround",
        CtxAction::ToggleFly => "ToggleFly",
        CtxAction::ResetCamera => "ResetCamera",
        CtxAction::Im(..) => "Im",
        CtxAction::Call(..) => "Call",
        CtxAction::ShowHistory(..) => "ShowHistory",
        CtxAction::OfferTeleport(..) => "OfferTeleport",
        CtxAction::AnswerTeleport { .. } => "AnswerTeleport",
        CtxAction::AnswerInventoryOffer { .. } => "AnswerInventoryOffer",
        CtxAction::Profile(..) => "Profile",
        CtxAction::SetRender(..) => "SetRender",
        CtxAction::ResetSkeleton { .. } => "ResetSkeleton",
        CtxAction::StopAnimations => "StopAnimations",
        CtxAction::StopAnimationsKeepPermissions => "StopAnimationsKeepPermissions",
        CtxAction::Undeform => "Undeform",
        CtxAction::RefreshAttachments => "RefreshAttachments",
        CtxAction::ToggleBlock(..) => "ToggleBlock",
        CtxAction::SetVoiceVolume(..) => "SetVoiceVolume",
        CtxAction::ToggleVoiceMute(..) => "ToggleVoiceMute",
        CtxAction::ModerateVoice(..) => "ModerateVoice",
        CtxAction::ToggleOwnerParticleFilter(..) => "ToggleOwnerParticleFilter",
        CtxAction::SetOwnerSoundBlock { .. } => "SetOwnerSoundBlock",
        CtxAction::InviteToGroup(..) => "InviteToGroup",
        CtxAction::GiveCallingCard(..) => "GiveCallingCard",
        CtxAction::CreateLandmark => "CreateLandmark",
        CtxAction::ReportAbuse(..) => "ReportAbuse",
        CtxAction::ToggleBlockObject { .. } => "ToggleBlockObject",
        CtxAction::Edit(..) => "Edit",
        CtxAction::Inspect(..) => "Inspect",
        CtxAction::InspectAvatar(..) => "InspectAvatar",
        CtxAction::Derender { .. } => "Derender",
        CtxAction::ScriptInfo(..) => "ScriptInfo",
        CtxAction::ObjectInfo(..) => "ObjectInfo",
        CtxAction::RefreshTextures(..) => "RefreshTextures",
        CtxAction::RefreshAvatarTextures(..) => "RefreshAvatarTextures",
        CtxAction::Rebake => "Rebake",
        CtxAction::Take(..) => "Take",
        CtxAction::TakeCopy(..) => "TakeCopy",
        CtxAction::Delete(..) => "Delete",
        CtxAction::DeleteConfirmed(..) => "DeleteConfirmed",
        CtxAction::Pay(..) => "Pay",
        CtxAction::PayResident(..) => "PayResident",
        CtxAction::Buy(..) => "Buy",
        CtxAction::Open(..) => "Open",
        CtxAction::Unlink(..) => "Unlink",
        CtxAction::Link(..) => "Link",
        CtxAction::TakeSeparate { .. } => "TakeSeparate",
        CtxAction::Return(..) => "Return",
        CtxAction::Wear { .. } => "Wear",
        CtxAction::Scripts { .. } => "Scripts",
        CtxAction::Drop(..) => "Drop",
        CtxAction::SetParticleBlock { .. } => "SetParticleBlock",
        CtxAction::AttachmentFavorite(..) => "AttachmentFavorite",
        CtxAction::EditLinkedParts(..) => "EditLinkedParts",
        CtxAction::Build => "Build",
        CtxAction::EditTerrain => "EditTerrain",
        CtxAction::DisplayName => "DisplayName",
        CtxAction::HoverHeight => "HoverHeight",
        CtxAction::OpenAppearance { .. } => "OpenAppearance",
        CtxAction::OpenPeople(..) => "OpenPeople",
        CtxAction::ZoomAvatar(..) => "ZoomAvatar",
        CtxAction::TeleportToAvatar(..) => "TeleportToAvatar",
        CtxAction::ShowOnMap(..) => "ShowOnMap",
        CtxAction::ShowPlaceInfo(..) => "ShowPlaceInfo",
        CtxAction::ShowPlace(..) => "ShowPlace",
        CtxAction::OpenUrl(..) => "OpenUrl",
        CtxAction::AppLink(..) => "AppLink",
        CtxAction::TeleportToPlace(..) => "TeleportToPlace",
        CtxAction::TeleportToPlaceConfirmed(..) => "TeleportToPlaceConfirmed",
        CtxAction::GrantRights { .. } => "GrantRights",
        CtxAction::AddFriend(..) => "AddFriend",
        CtxAction::RemoveFriend(..) => "RemoveFriend",
        CtxAction::RemoveFriends(..) => "RemoveFriends",
        CtxAction::Conference(..) => "Conference",
        CtxAction::ConferenceCall(..) => "ConferenceCall",
        CtxAction::OfferTeleports(..) => "OfferTeleports",
        CtxAction::RequestTeleport(..) => "RequestTeleport",
        CtxAction::GroupChat(..) => "GroupChat",
        CtxAction::ActivateGroup(..) => "ActivateGroup",
        CtxAction::PinGroup(..) => "PinGroup",
        CtxAction::LeaveGroup(..) => "LeaveGroup",
        CtxAction::GroupChatBlocked(..) => "GroupChatBlocked",
        CtxAction::ContactSet(..) => "ContactSet",
        CtxAction::NewContactSet(..) => "NewContactSet",
    };
    match action {
        CtxAction::TakeOffClothing(kind) | CtxAction::DetachAt(kind) => format!("{base}:{kind}"),
        CtxAction::Derender { permanent, .. } => format!("{base}:{permanent}"),
        CtxAction::SetRender(_, mode) | CtxAction::OpenPeople(mode) => format!("{base}:{mode}"),
        CtxAction::SetOwnerSoundBlock { flag, .. } => format!("{base}:{flag}"),
        CtxAction::SetParticleBlock { owner, .. } => format!("{base}:{owner}"),
        CtxAction::ModerateVoice(Some(_), _) => format!("{base}:selected"),
        CtxAction::ModerateVoice(None, muted) => format!("{base}:all:{muted}"),
        CtxAction::ResetSkeleton { animations, .. } => format!("{base}:{animations}"),
        CtxAction::TakeSeparate { copy, .. } => format!("{base}:{copy}"),
        CtxAction::Wear { point, add, .. } => format!("{base}:{point}:{add}"),
        CtxAction::Scripts { action, .. } => format!("{base}:{action:?}"),
        CtxAction::OpenAppearance { tab, editing } => format!("{base}:{tab}:{editing}"),
        _ => base.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use uuid::Uuid;

    fn menu(id: u128) -> ContextMenu {
        ContextMenu {
            target: Target::Object {
                key: crate::world::objects::ObjKey {
                    region: 1,
                    local_id: id as u32,
                },
                full_id: Uuid::from_u128(id),
                offset: Vec3::ZERO,
            },
            pos: egui::pos2(80.0, 80.0),
            point: Vec3::ZERO,
            pie_path: Vec::new(),
            pie_input_frame: None,
        }
    }

    #[test]
    fn scene_menu_profiles_separate_huds_attachments_and_every_pick_kind() {
        let mut facts = Facts::default();
        let own = Target::Attachment {
            key: crate::world::objects::ObjKey { region: 1, local_id: 3 },
        };
        let mut scopes = vec![scope(&Target::Ground, &facts), scope(&menu(1).target, &facts), scope(&own, &facts)];
        facts.object.local_hud_scale = Some(1.0);
        scopes.push(scope(&own, &facts));
        for (own, attachment) in [(true, None), (false, None), (false, Some(Uuid::from_u128(2)))] {
            scopes.push(scope(
                &Target::Avatar {
                    id: Uuid::from_u128(1),
                    own,
                    attachment,
                },
                &facts,
            ));
        }
        scopes.sort_unstable();
        scopes.dedup();
        assert_eq!(scopes.len(), 7);
    }

    #[test]
    fn scene_menu_keys_keep_toggle_identity_and_distinguish_parameterized_commands() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(action_key(&CtxAction::Edit(a)), action_key(&CtxAction::Edit(b)));
        assert_eq!(
            action_key(&CtxAction::ModerateVoice(Some(a), false)),
            action_key(&CtxAction::ModerateVoice(Some(b), true))
        );
        assert_ne!(
            action_key(&CtxAction::ModerateVoice(None, false)),
            action_key(&CtxAction::ModerateVoice(None, true))
        );
        assert_ne!(
            action_key(&CtxAction::ModerateVoice(Some(a), true)),
            action_key(&CtxAction::ModerateVoice(None, true))
        );
        assert_eq!(
            action_key(&CtxAction::SetPropertyLines(false)),
            action_key(&CtxAction::SetPropertyLines(true))
        );
        assert_ne!(action_key(&CtxAction::SetRender(a, 1)), action_key(&CtxAction::SetRender(a, 2)));
        assert_ne!(action_key(&CtxAction::DetachAt(1)), action_key(&CtxAction::DetachAt(2)));
        assert_ne!(
            action_key(&CtxAction::TakeOffClothing(4)),
            action_key(&CtxAction::TakeOffClothing(5))
        );
        assert_eq!(
            action_key(&CtxAction::SetOwnerSoundBlock {
                owner: a,
                flag: 1,
                blocked: false
            }),
            action_key(&CtxAction::SetOwnerSoundBlock {
                owner: b,
                flag: 1,
                blocked: true
            })
        );
        let target = Target::Attachment {
            key: crate::world::objects::ObjKey { region: 1, local_id: 3 },
        };
        let sit_stand_id = |seated| {
            let facts = Facts {
                seated,
                ..Default::default()
            };
            Catalog::new(&entries::world_menu(&target, &facts, Vec3::ZERO))
                .originals
                .into_iter()
                .find(|(_, entry)| entry.label == aurora_i18n::tr!("ui-context-sit-stand"))
                .expect("Sit / Stand row")
                .0
        };
        assert_eq!(sit_stand_id(false), sit_stand_id(true), "posture never moves a customized toggle");
    }

    #[test]
    fn scene_menu_saved_orders_use_current_identity_and_current_permissions() {
        let facts = Facts {
            object: super::super::ObjectFacts {
                edit: true,
                id: Uuid::from_u128(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let first = menu(1);
        let before = entries::world_menu(&first.target, &facts, Vec3::ZERO);
        let catalog = Catalog::new(&before);
        let edit = catalog.entries.iter().find(|e| matches!(e.kind, layout::Kind::Action(i) if matches!(catalog.commands[i].kind, entries::Kind::Action(CtxAction::Edit(_))))).expect("edit row");
        let mut prefs = Preferences::default();
        profile(&mut prefs, "scene:object")
            .layout
            .placements
            .insert(edit.id.clone(), "top".into());
        let saved = serde_json::to_vec(&prefs).expect("save");
        let loaded: Preferences = serde_json::from_slice(&saved).expect("load");
        assert_eq!(loaded, prefs);
        let second = menu(2);
        let after = entries::world_menu(&second.target, &Facts::default(), Vec3::ZERO);
        let (radial, customized) = radial_entries(&second, &Facts::default(), &after, &loaded);
        assert!(customized);
        let mut actions = Vec::new();
        fn collect(rows: &[entries::Entry], out: &mut Vec<CtxAction>) {
            for row in rows {
                match &row.kind {
                    entries::Kind::Action(action) => out.push(action.clone()),
                    entries::Kind::Submenu(children) => collect(children, out),
                    _ => {}
                }
            }
        }
        collect(&radial, &mut actions);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, CtxAction::Edit(id) if *id == Uuid::from_u128(2)))
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, CtxAction::Edit(id) if *id == Uuid::from_u128(1)))
        );
        let pbr = radial
            .iter()
            .find(|row| matches!(row.kind, entries::Kind::Action(CtxAction::EditPbr(_))))
            .expect("PBR");
        assert!(!pbr.enabled);
        assert!(!loaded.profiles.contains_key("scene:land"));
    }

    #[test]
    fn scene_menu_hiding_an_action_applies_to_radial_too_and_keeps_editor_reachable() {
        let m = menu(1);
        let facts = Facts::default();
        let entries = entries::world_menu(&m.target, &facts, Vec3::ZERO);
        let catalog = Catalog::new(&entries);
        let zoom = catalog.entries.iter().find(|entry| matches!(entry.kind, layout::Kind::Action(i) if matches!(catalog.commands[i].kind, entries::Kind::Action(CtxAction::Zoom(..))))).expect("zoom");
        let mut prefs = Preferences::default();
        profile(&mut prefs, scope(&m.target, &facts))
            .layout
            .placements
            .insert(zoom.id.clone(), layout::HIDDEN.into());
        let (rows, _) = radial_entries(&m, &facts, &entries, &prefs);
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.kind, entries::Kind::Action(CtxAction::Zoom(..))))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, entries::Kind::Action(CtxAction::CustomizeMenu(_))))
        );
    }

    #[test]
    fn scene_menu_radial_icon_bars_have_a_label_and_follow_absolute_slot_order() {
        let m = menu(1);
        let facts = Facts::default();
        let entries = entries::world_menu(&m.target, &facts, Vec3::ZERO);
        let catalog = Catalog::new(&entries);
        let mut prefs = Preferences::default();
        let profile = profile(&mut prefs, "scene:object");
        for entry in &catalog.entries {
            let layout::Kind::Action(index) = entry.kind else { continue };
            let slot = match catalog.commands[index].kind {
                entries::Kind::Action(CtxAction::Zoom(..)) => 0,
                entries::Kind::Action(CtxAction::Edit(_)) => 7,
                _ => continue,
            };
            profile.layout.placements.insert(entry.id.clone(), "top".into());
            profile.layout.slots.entry("top".into()).or_default().insert(entry.id.clone(), slot);
        }
        let (rows, _) = radial_entries(&m, &facts, &entries, &prefs);
        let bar = rows
            .iter()
            .find(|row| row.label == aurora_i18n::tr!("inventory-menu-icon-bar"))
            .expect("named bar");
        let entries::Kind::Submenu(children) = &bar.kind else {
            panic!("bar submenu")
        };
        assert!(matches!(children[0].kind, entries::Kind::Action(CtxAction::Zoom(..))));
        assert!(matches!(children[1].kind, entries::Kind::Action(CtxAction::Edit(_))));
    }

    #[test]
    fn scene_menu_editing_never_executes_and_dismissal_relocks() {
        use crate::ui::widgets::elide_tests::Frames;
        let mut frames = Frames::new();
        let ctx = frames.ctx.clone();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let m = menu(1);
        let mut current = Some(m.clone());
        let mut prefs = Preferences::default();
        let mut state = Customization::default();
        state.edit(&ctx, &m, &Facts::default(), &mut prefs);
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = frames.run(Vec::new(), |ui| {
                super::super::show(ui.ctx(), &p, &mut current, &Facts::default(), false, &mut prefs, &mut state);
            });
        }
        let target = texts
            .iter()
            .find(|text| text.source == aurora_i18n::tr!("common-build"))
            .expect("Build")
            .rect
            .center();
        for pressed in [true, false] {
            frames.run(
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                |ui| {
                    super::super::show(ui.ctx(), &p, &mut current, &Facts::default(), false, &mut prefs, &mut state);
                },
            );
        }
        assert!(super::super::take_requests(&ctx).is_empty());
        current = None;
        frames.run(Vec::new(), |ui| {
            super::super::show(ui.ctx(), &p, &mut current, &Facts::default(), false, &mut prefs, &mut state);
        });
        assert!(prefs.profiles["scene:object"].locked);
    }
}
