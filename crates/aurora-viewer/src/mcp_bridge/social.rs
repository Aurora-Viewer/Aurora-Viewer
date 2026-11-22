//! Phase 5 (C4) MCP social & teleport handlers: IM sessions, conference chat,
//! typing indicators, social offers (lures/friends/groups), teleport offers,
//! landmarks, and Home location.

use super::*;
use crate::world::notifications::{Data, Response};

pub(super) struct PendingLandmark {
    name: String,
    parent: Uuid,
    before_ids: HashSet<Uuid>,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

impl App {
    pub(super) fn handle_mcp_im_sessions_list(&self) -> ViewerResponse {
        let mut sessions: Vec<Value> = self
            .world
            .social
            .ims
            .iter()
            .map(|s| {
                let group_info = self.world.groups.sessions.get(&s.other);
                let kind = if self.world.groups.is_member(&s.other) || group_info.is_some_and(|g| g.group) {
                    "group"
                } else if group_info.is_some() {
                    "conference"
                } else {
                    "im"
                };
                let name = group_info
                    .filter(|g| !g.name.is_empty())
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| self.world.social.name_of(&s.other));
                json!({
                    "session_id": s.other.to_string(),
                    "kind": kind,
                    "name": name,
                    "unread": s.unread,
                    "message_count": s.lines.len(),
                    "typing": self.world.social.is_typing(&s.other),
                    "active": group_info.map(|g| g.active),
                    "closed": group_info.map(|g| g.closed),
                    "text_muted": group_info.map(|g| g.text_muted),
                    "participants": group_info.map(|g| g.participants.iter().map(ToString::to_string).collect::<Vec<_>>()),
                })
            })
            .collect();

        // Include group/conference sessions that might not yet have a 1:1 `ImSession` entry.
        for (&sid, g) in &self.world.groups.sessions {
            if !self.world.social.ims.iter().any(|s| s.other == sid) {
                sessions.push(json!({
                    "session_id": sid.to_string(),
                    "kind": if g.group { "group" } else { "conference" },
                    "name": g.name,
                    "unread": 0,
                    "message_count": 0,
                    "typing": false,
                    "active": g.active,
                    "closed": g.closed,
                    "text_muted": g.text_muted,
                    "participants": g.participants.iter().map(ToString::to_string).collect::<Vec<_>>(),
                }));
            }
        }

        ViewerResponse::Json(json!({
            "ok": true,
            "count": sessions.len(),
            "sessions": sessions,
        }))
    }

    pub(super) fn handle_mcp_im_session_read(&self, session_id: Uuid, limit: usize) -> Result<ViewerResponse, McpError> {
        let Some(sess) = self.world.social.ims.iter().find(|s| s.other == session_id) else {
            if self.world.groups.sessions.contains_key(&session_id) {
                return Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "session_id": session_id.to_string(),
                    "count": 0,
                    "messages": [],
                })));
            }
            return Err(McpError::InvalidParams(format!("no open IM session '{session_id}'")));
        };
        let skip = sess.lines.len().saturating_sub(limit);
        let messages: Vec<Value> = sess
            .lines
            .iter()
            .skip(skip)
            .map(|l| {
                let ts_ms = l.time.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                json!({
                    "timestamp_unix_ms": ts_ms,
                    "from": l.from,
                    "source_id": l.source.to_string(),
                    "text": l.text,
                    "kind": format!("{:?}", l.kind),
                })
            })
            .collect();
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "session_id": session_id.to_string(),
            "unread": sess.unread,
            "typing": self.world.social.is_typing(&session_id),
            "count": messages.len(),
            "messages": messages,
        })))
    }

    pub(super) fn handle_mcp_im_session_send(
        &mut self,
        session_id: Option<Uuid>,
        group_id: Option<Uuid>,
        target_uuid: Option<Uuid>,
        participants: Vec<Uuid>,
        message: Option<String>,
        action: &str,
    ) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        if action.eq_ignore_ascii_case("leave") {
            let sid = session_id
                .or(group_id)
                .ok_or_else(|| McpError::InvalidParams("leaving requires 'session_id' or 'group_id'".into()))?;
            self.world.leave_session(sid);
            for cmd in self.world.groups.take_commands() {
                self.mcp_send(cmd)?;
            }
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "action": "leave",
                "session_id": sid.to_string(),
            })));
        }

        let text = message
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| McpError::InvalidParams("missing non-empty 'message'".into()))?;

        // 1. Ad-hoc conference creation/send.
        if !participants.is_empty() {
            if !self.world.rlva_start_ims(&participants) {
                return Err(McpError::PermissionDenied("RLVa prevents starting IM conversations".into()));
            }
            self.world.start_text_conference(participants.clone());
            let conf_session = self.world.social.focus_im;
            if let Some(sid) = conf_session {
                self.world.send_session_im(sid, text);
            }
            for cmd in self.world.groups.take_commands() {
                self.mcp_send(cmd)?;
            }
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "kind": "conference",
                "session_id": conf_session.map(|u| u.to_string()),
                "participants_count": participants.len(),
                "sent": true,
            })));
        }

        // 2. Group chat or active group/conference session.
        let group_target =
            group_id.or_else(|| session_id.filter(|sid| self.world.groups.sessions.contains_key(sid) || self.world.groups.is_member(sid)));
        if let Some(gid) = group_target {
            if self.world.groups.is_member(&gid) && !self.world.groups.sessions.contains_key(&gid) {
                if !self.world.rlva_start_ims(&[gid]) {
                    return Err(McpError::PermissionDenied("RLVa prevents group IM".into()));
                }
                self.world.start_group_chat(gid, false);
            }
            self.world.send_session_im(gid, text);
            for cmd in self.world.groups.take_commands() {
                self.mcp_send(cmd)?;
            }
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "kind": if self.world.groups.is_member(&gid) { "group" } else { "conference" },
                "session_id": gid.to_string(),
                "sent": true,
            })));
        }

        // 3. Direct 1:1 IM.
        let to = target_uuid
            .or(session_id)
            .ok_or_else(|| McpError::InvalidParams("provide 'target_uuid', 'group_id', 'session_id', or 'participants'".into()))?;
        if !self.world.rlva_can_start_im(to) {
            return Err(McpError::PermissionDenied("RLVa prevents sending IM to that resident".into()));
        }
        self.mcp_send(NetCommand::SendIm {
            to,
            message: text.to_owned(),
        })?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "kind": "im",
            "target_uuid": to.to_string(),
            "sent": true,
        })))
    }

    pub(super) fn handle_mcp_im_typing(
        &mut self,
        target_uuid: Option<Uuid>,
        session_id: Option<Uuid>,
        typing: bool,
    ) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        if let Some(to) = target_uuid {
            if !self.world.rlva_can_start_im(to) {
                return Err(McpError::PermissionDenied("RLVa prevents IM to that resident".into()));
            }
            let sid = session_id.unwrap_or_else(|| Uuid::from_u128(to.as_u128() ^ self.world.agent_id.as_u128()));
            let dialog = if typing {
                crate::world::IM_TYPING_START
            } else {
                crate::world::IM_TYPING_STOP
            };
            self.mcp_send(NetCommand::SendImDialog {
                to,
                dialog,
                id: sid,
                message: "typing".into(),
                bucket: Vec::new(),
            })?;
            Ok(ViewerResponse::Json(json!({
                "ok": true,
                "scope": "im",
                "target_uuid": to.to_string(),
                "typing": typing,
            })))
        } else {
            Ok(ViewerResponse::Json(json!({
                "ok": true,
                "scope": "local",
                "typing": typing,
            })))
        }
    }

    pub(super) fn handle_mcp_social_offers_list(&self) -> ViewerResponse {
        let offers: Vec<Value> = self
            .world
            .notifications
            .list
            .iter()
            .filter_map(|n| match &n.data {
                Data::Lure {
                    from,
                    invite,
                    change_rating,
                } => Some(json!({
                    "notification_id": n.id,
                    "kind": "teleport_lure",
                    "from_id": from.to_string(),
                    "from_name": self.world.social.name_of(from),
                    "invite_id": invite,
                    "change_rating": change_rating,
                    "title": n.title,
                    "message": n.body,
                })),
                Data::TeleportRequest { from, invite } => Some(json!({
                    "notification_id": n.id,
                    "kind": "teleport_request",
                    "from_id": from.to_string(),
                    "from_name": self.world.social.name_of(from),
                    "invite_id": invite,
                    "title": n.title,
                    "message": n.body,
                })),
                Data::Friend { from, tx, offline } => Some(json!({
                    "notification_id": n.id,
                    "kind": "friendship_offer",
                    "from_id": from.to_string(),
                    "from_name": self.world.social.name_of(from),
                    "transaction_id": tx.to_string(),
                    "offline": offline,
                    "title": n.title,
                    "message": n.body,
                })),
                Data::Group {
                    group, tx, offline, fee, ..
                } => Some(json!({
                    "notification_id": n.id,
                    "kind": "group_invitation",
                    "group_id": group.to_string(),
                    "transaction_id": tx.to_string(),
                    "offline": offline,
                    "fee": fee,
                    "title": n.title,
                    "message": n.body,
                })),
                _ => None,
            })
            .collect();

        ViewerResponse::Json(json!({
            "ok": true,
            "count": offers.len(),
            "offers": offers,
        }))
    }

    pub(super) fn handle_mcp_social_offer_respond(&mut self, notification_id: u64, accept: bool) -> Result<ViewerResponse, McpError> {
        let kind = self
            .world
            .notifications
            .list
            .iter()
            .find(|n| n.id == notification_id)
            .and_then(|n| match &n.data {
                Data::Lure { from, .. } => Some(("teleport_lure", *from)),
                Data::TeleportRequest { from, .. } => Some(("teleport_request", *from)),
                Data::Friend { from, .. } => Some(("friendship_offer", *from)),
                Data::Group { group, .. } => Some(("group_invitation", *group)),
                _ => None,
            })
            .ok_or_else(|| {
                McpError::InvalidParams(format!(
                    "no pending social offer with notification_id {notification_id}; check social_offers_list"
                ))
            })?;

        if accept && kind.0 == "teleport_lure" && !self.world.rlva_can_accept_teleport(kind.1, false) {
            return Err(McpError::PermissionDenied("RLVa prevents accepting teleport lures".into()));
        }
        if accept && kind.0 == "teleport_request" && !self.world.rlva_can_offer_teleport(kind.1) {
            return Err(McpError::PermissionDenied("RLVa prevents offering teleports".into()));
        }

        let (commands, error_note) = self
            .world
            .respond_notification(notification_id, if accept { Response::Accept } else { Response::Decline });
        if let Some(err) = error_note {
            return Err(McpError::PermissionDenied(err));
        }
        for cmd in commands {
            self.mcp_send(cmd)?;
        }
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "notification_id": notification_id,
            "kind": kind.0,
            "accepted": accept,
        })))
    }

    pub(super) fn handle_mcp_teleport_offer(
        &mut self,
        target_uuids: Vec<Uuid>,
        message: String,
        mode: &str,
    ) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        for &id in &target_uuids {
            if !self.world.rlva_can_offer_teleport(id) {
                return Err(McpError::PermissionDenied(format!(
                    "RLVa prevents offering/requesting teleport with {id}"
                )));
            }
        }
        if mode.eq_ignore_ascii_case("request") {
            const IM_TELEPORT_REQUEST: u8 = 26;
            for &to in &target_uuids {
                self.mcp_send(NetCommand::SendImDialog {
                    to,
                    dialog: IM_TELEPORT_REQUEST,
                    id: Uuid::new_v4(),
                    message: message.clone(),
                    bucket: Vec::new(),
                })?;
            }
        } else {
            self.mcp_send(NetCommand::OfferTeleport {
                to: target_uuids.clone(),
                message,
            })?;
        }
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "mode": mode,
            "targets": target_uuids.into_iter().map(|u| u.to_string()).collect::<Vec<_>>(),
        })))
    }

    pub(super) fn start_mcp_landmark_create(
        &mut self,
        name: Option<String>,
        folder_id: Option<Uuid>,
        folder_path: Option<String>,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if !self.in_world() || self.world.parcel.is_none() {
            let _ = reply_tx.send(Err(McpError::NotInWorld));
            return;
        }
        if self.world.rlva.has("showloc") {
            let _ = reply_tx.send(Err(McpError::PermissionDenied("RLVa @showloc prevents creating landmarks".into())));
            return;
        }
        let inv = &self.world.inventory;
        let parent = match (folder_id, folder_path.as_deref()) {
            (Some(id), _) => {
                if !inv.folders.contains_key(&id) {
                    let _ = reply_tx.send(Err(McpError::InvalidParams(format!("unknown folder_id {id}"))));
                    return;
                }
                id
            }
            (None, Some(path)) => match self.resolve_inventory_path(path) {
                Ok(id) => id,
                Err(e) => {
                    let _ = reply_tx.send(Err(e));
                    return;
                }
            },
            (None, None) => crate::world::inventory::actions::system(inv, 3).unwrap_or(inv.root),
        };
        let default_name = format!(
            "{} ({:.0}, {:.0}, {:.0})",
            self.world.region_name(),
            self.world.agent.position.x,
            self.world.agent.position.y,
            self.world.agent.position.z
        );
        let final_name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).unwrap_or(default_name);
        let before_ids: HashSet<Uuid> = self
            .world
            .inventory
            .items
            .values()
            .filter(|it| it.parent == parent && it.asset_type == 3)
            .map(|it| it.id)
            .collect();

        if let Err(e) = self.mcp_send(NetCommand::CreateInventoryItem {
            parent,
            kind: aurora_net::inventory::operations::NewItem::Landmark,
            name: final_name.clone(),
            permissions: None,
        }) {
            let _ = reply_tx.send(Err(e));
            return;
        }

        self.mcp_bridge.pending_landmarks.push_back(PendingLandmark {
            name: final_name,
            parent,
            before_ids,
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(500, 15_000)),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_landmark_create(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_landmarks) {
            let created = self
                .world
                .inventory
                .items
                .values()
                .filter(|it| it.parent == p.parent && it.asset_type == 3 && !p.before_ids.contains(&it.id))
                .max_by_key(|it| u8::from(it.name == p.name));
            if let Some(item) = created {
                let ids = crate::world::inventory::asset_ids::AssetIds::of(&self.world);
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "verified": true,
                    "item_id": item.id.to_string(),
                    "asset_id": ids.id(item.asset_id).to_string(),
                    "folder_id": p.parent.to_string(),
                    "name": item.name,
                }))));
            } else if now.duration_since(p.started) >= p.deadline {
                let _ = p.reply_tx.send(Err(McpError::Timeout(p.deadline.as_millis() as u64)));
            } else {
                self.mcp_bridge.pending_landmarks.push_back(p);
            }
        }
    }

    pub(super) fn handle_mcp_teleport_landmark(&mut self, item_id: Uuid) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        if !self.world.rlva_can_teleport_landmark() {
            return Err(McpError::PermissionDenied("RLVa prevents teleporting via landmark".into()));
        }
        let Some(item) = self.world.inventory.items.get(&item_id).cloned() else {
            return Err(McpError::InvalidParams(format!("no landmark item {item_id} in inventory")));
        };
        if item.asset_type != 3 || item.asset_id.is_nil() {
            return Err(McpError::InvalidParams(format!("'{}' is not a valid landmark item", item.name)));
        }
        self.mcp_send(NetCommand::TeleportLandmark(item.asset_id))?;
        let ids = crate::world::inventory::asset_ids::AssetIds::of(&self.world);
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "status": "teleporting_to_landmark",
            "item_id": item_id.to_string(),
            "asset_id": ids.id(item.asset_id).to_string(),
            "name": item.name,
        })))
    }

    pub(super) fn handle_mcp_set_home(&mut self) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        let position = self.world.agent.position;
        let look_at = (self.world.agent.body_rotation() * Vec3::X).normalize_or(Vec3::X);
        self.mcp_send(NetCommand::SetHomeLocation { position, look_at })?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "status": "home_location_sent",
            "position": [position.x, position.y, position.z],
            "look_at": [look_at.x, look_at.y, look_at.z],
        })))
    }

    pub(super) fn handle_mcp_teleport_home(&mut self) -> Result<ViewerResponse, McpError> {
        if !self.in_world() {
            return Err(McpError::NotInWorld);
        }
        if !self.world.rlva_can_teleport_home() {
            return Err(McpError::PermissionDenied("RLVa prevents teleporting home".into()));
        }
        self.mcp_send(NetCommand::TeleportHome)?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "status": "teleporting_home",
        })))
    }
}
