//! "Derender temporarily" / "Derender & blacklist" of the menus and the
//! Asset blacklist window: what is derendered, under which name, and the
//! requests for what comes back (world/derender.rs, ui/blacklist.rs).
//! Follows derenderObject (indra/newview/llviewermenu.cpp) and
//! LLAvatarActions::derender (indra/newview/llavataractions.cpp).
use super::*;
use crate::ui::context::CtxAction;
use crate::world::derender::{Entry, Kind};
use crate::world::objects::ObjKey;
use uuid::Uuid;

impl App {
    /// derenderObject on the menu's object, and on every other linkset of
    /// the selection when the object is one of them (the Legacy Viewer
    /// takes each selected root), or LLAvatarActions::derender on an
    /// avatar, in view or only in a list.
    pub(super) fn derender(&mut self, id: Uuid, permanent: bool) {
        let w = &self.world;
        let targets = match w.objects.index_of_uuid(&id) {
            Some(idx) if w.objects.get(idx).is_some_and(|o| !o.is_avatar()) => self
                .build
                .menu_roots(w, idx)
                .into_iter()
                .filter_map(|r| w.objects.get(r))
                .filter(|o| !o.is_avatar())
                .map(|o| o.full_id)
                .collect(),
            _ => vec![id],
        };
        for id in targets {
            self.derender_one(id, permanent);
        }
    }

    /// derenderObject on one linkset or worn object (from its root), or
    /// LLAvatarActions::derender on an avatar.
    fn derender_one(&mut self, id: Uuid, permanent: bool) {
        let w = &self.world;
        if id == w.agent_id || id.is_nil() {
            return;
        }
        let idx = w.objects.index_of_uuid(&id);
        let avatar = idx.and_then(|i| w.objects.get(i)).is_none_or(|o| o.is_avatar());
        let (target, entry) = if avatar {
            // FirstName / LastName of the avatar, or its user name
            let name = w.legacy_name(&id).unwrap_or_else(|| w.social.name_of(&id));
            let entry = Entry {
                name,
                region: String::new(),
                kind: Kind::Resident,
                date: crate::world::derender::now_secs(),
                permanent,
            };
            (id, entry)
        } else {
            let Some(root) = idx.map(|i| context_menu::linkset_root(w, i)) else {
                return;
            };
            let Some(o) = w.objects.get(root) else {
                return;
            };
            // the selection's root node name (ObjectProperties)
            let name = self
                .build
                .props
                .get(&o.full_id)
                .or_else(|| self.interactions.props.get(&o.full_id))
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let region = w.regions.get(&o.key.region).map(|r| r.name.clone()).unwrap_or_default();
            let entry = Entry {
                name,
                region,
                kind: Kind::Object,
                date: crate::world::derender::now_secs(),
                permanent,
            };
            (o.full_id, entry)
        };
        // deselectObjectOnly, and the tools stop editing it
        if let Some(root) = self.world.objects.index_of_uuid(&target) {
            let keys: Vec<ObjKey> = self
                .world
                .objects
                .subtree(root)
                .into_iter()
                .filter_map(|i| self.world.objects.get(i).map(|o| o.key))
                .filter(|k| self.build.is_selected(k))
                .collect();
            if !keys.is_empty() {
                self.build.deselect(&self.world, &keys);
                self.flush_build();
            }
        }
        // no name yet (an object worn by another is not selected by its
        // menu): ObjectPropertiesFamily gives it, see send_derender_requests
        if !avatar
            && entry.name.is_empty()
            && let Some(o) = self.world.objects.index_of_uuid(&target).and_then(|i| self.world.objects.get(i))
        {
            let handle = o.key.region;
            self.send(NetCommand::RequestObjectProperties { handle, object: target });
        }
        log::info!("derender {target} (permanent: {permanent})");
        if self.world.derender(target, entry) {
            // we sat on it: gAgent.standUp()
            self.send(NetCommand::OneShotControl(control::STAND_UP));
        }
    }

    /// Show Friends Only: the account's choice loaded after a login, the
    /// changes kept in the settings (per account, FSRenderFriendsOnly and
    /// FSRenderFriendsOnlyPersistsTP), and the friendships followed.
    pub(super) fn sync_friends_only(&mut self) {
        let id = self.world.agent_id;
        if id.is_nil() {
            return;
        }
        if self.friends_only_account != id || std::mem::take(&mut self.world.friends_only.reload) {
            self.friends_only_account = id;
            let saved = self.settings.friends_only.get(&id).copied().unwrap_or_default();
            self.world.friends_only.keep_after_teleport = saved.keep_after_teleport;
            self.world.set_friends_only(saved.enabled);
        }
        self.world.follow_friends_for_friends_only();
        let now = crate::world::friends_only::Preferences {
            enabled: self.world.friends_only(),
            keep_after_teleport: self.world.friends_only.keep_after_teleport,
        };
        let saved = self.settings.friends_only.get(&id).copied().unwrap_or_default();
        if now != saved {
            if now == Default::default() {
                self.settings.friends_only.remove(&id);
            } else {
                self.settings.friends_only.insert(id, now);
            }
            self.settings.save();
            log::info!(
                "show friends only: {} (kept after a teleport: {})",
                now.enabled,
                now.keep_after_teleport
            );
        }
    }

    /// The Asset blacklist window, while open.
    pub(super) fn blacklist_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.panels.asset_blacklist {
            return;
        }
        let mut open = true;
        let ids = crate::world::inventory::asset_ids::AssetIds::per_frame(&self.world);
        let action = self.blacklist_ui.show(ctx, p, &self.world.derender, &ids, &mut open);
        self.panels.asset_blacklist = open;
        match action {
            Some(ui::blacklist::BlacklistAction::OpenIgnored) => self.open_ignored_list(),
            Some(ui::blacklist::BlacklistAction::Remove(rows)) => self.world.remove_blacklist_rows(&rows),
            Some(ui::blacklist::BlacklistAction::ClearTemporary) => self.world.clear_temporary_derenders(),
            None => {}
        }
    }

    pub(super) fn open_ignored_list(&mut self) {
        self.people_ui.filter.clear();
        self.panels.people_tab = ui::people::TAB_BLOCKED;
        ui::widgets::open_floater(&self.egui_ctx, "people", &mut self.panels.people);
    }

    /// RequestMultipleObjects for the objects back from the list, and the
    /// names of the objects listed without one, once their properties came.
    pub(super) fn send_derender_requests(&mut self) {
        for (handle, local_ids) in self.world.derender.take_requests() {
            self.send(NetCommand::RequestObjects { handle, local_ids });
        }
        for id in self.world.derender.unnamed() {
            let props = self.build.props.get(&id).or_else(|| self.interactions.props.get(&id));
            if let Some(name) = props.map(|p| p.name.clone()).filter(|n| !n.is_empty()) {
                self.world.derender.rename(&id, &name);
            }
        }
    }

    /// AURORA_DEMO_FRIENDS_ONLY: Show Friends Only switched on at frame
    /// 700, then off at 900 (docs/TESTING.md).
    pub(super) fn demo_friends_only_steps(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_FRIENDS_ONLY") else {
            return;
        };
        let wolf = crate::demo::DEMO_LOUP;
        let friend = mode.trim() == "friend";
        match self.frame_count {
            // its look (shape and bakes), as an avatar of the grid has: it
            // must come back dressed (the plain demo's wolf has none)
            5 => {
                if let Some(look) = crate::demo::classic_events(&self.world.avatar_lib.shape_params).into_iter().nth(1) {
                    let _ = self.world.apply(look);
                }
            }
            // (the wolf is a friend: it stays, until the friendship ends)
            650 if friend => self.world.social.form_friendship(wolf),
            700 => self.world.set_friends_only(true),
            800 if friend => {
                self.world.social.end_friendship(&wolf);
            }
            900 => self.world.set_friends_only(false),
            _ => {}
        }
    }

    /// AURORA_DEMO_DERENDER: the scripted right click on the demo's other
    /// avatar (or the hat it wears) and the entries chosen
    /// (docs/TESTING.md).
    pub(super) fn demo_derender_steps(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_DERENDER") else {
            return;
        };
        let mode = mode.trim();
        let wolf = crate::demo::DEMO_LOUP;
        let hat = crate::demo::wolf_hat();
        // the object right-clicked: the hat for "hat", else the avatar
        let target = if mode == "hat" { hat } else { wolf };
        match self.frame_count {
            // the menu a right click on it opens (our own avatar may stand
            // in front of it from the start position)
            700 => {
                let now = Instant::now();
                if let Some(idx) = self.world.objects.index_of_uuid(&target)
                    && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, now, 0)
                    && let Some((x, y)) = self.build.cam.project_px(pos)
                {
                    let ppp = self.egui_ctx.pixels_per_point().max(0.1);
                    self.context_menu = Some(ui::context::ContextMenu {
                        pos: egui::pos2(x / ppp, y / ppp),
                        point: pos,
                        target: ui::context::Target::Avatar {
                            id: wolf,
                            own: false,
                            attachment: (mode == "hat").then_some(hat),
                        },
                        pie_path: Vec::new(),
                        pie_input_frame: None,
                    });
                }
            }
            740 => {
                let action = match mode {
                    "inspect" => Some(CtxAction::InspectAvatar(wolf)),
                    "avatar" => Some(CtxAction::Derender {
                        id: wolf,
                        permanent: false,
                    }),
                    "hat" => Some(CtxAction::Derender { id: hat, permanent: true }),
                    _ => None,
                };
                if let Some(action) = action {
                    self.context_menu = None;
                    self.build.release_menu_selection(&self.world);
                    self.on_ctx_action(action);
                }
            }
            // the window shows what is derendered
            800 if matches!(mode, "avatar" | "hat") => {
                ui::widgets::open_floater(&self.egui_ctx, ui::blacklist::WINDOW, &mut self.panels.asset_blacklist);
            }
            // and puts it back: "Clear temporary", or "Remove selected"
            900 if mode == "avatar" => self.world.clear_temporary_derenders(),
            900 if mode == "hat" => self.world.underender(&[hat]),
            _ => {}
        }
    }
}
