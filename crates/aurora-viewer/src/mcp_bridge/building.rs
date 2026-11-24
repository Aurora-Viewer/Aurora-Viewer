//! Phase 5 (C5) MCP building, scripting, and material handlers:
//! `prim_duplicate`, `prim_select`, `linkset_transform`, `build_transaction`,
//! `script_reset`, `script_set_running`, `prim_set_gltf_material`,
//! `prim_set_media`, and `prim_set_sculpt`.

use super::*;
use crate::build::materials::GltfUpdate;
use crate::media::entry::MediaEntry;
use aurora_assets::material::{AlphaMode, PbrOverride, TextureTransform};
use aurora_net::RegionHandle;
use aurora_net::build::{BuildCmd, TransformUpdate, perm};
use aurora_prim::extra::{
    FlexibleParams, LightParams, PARAMS_FLEXIBLE, PARAMS_LIGHT, PARAMS_SCULPT, pack_flexible, pack_light, pack_sculpt,
};
use aurora_prim::params::{LL_SCULPT_FLAG_INVERT, LL_SCULPT_FLAG_MIRROR, SculptParams};

pub(super) struct PendingDuplicate {
    region: RegionHandle,
    known_ids: HashSet<u32>,
    expected_count: usize,
    last_polled: Option<Instant>,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

#[derive(Debug, Clone)]
pub(super) struct GroupTransformTarget {
    pub local_id: u32,
    pub is_root: bool,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Debug, Clone)]
struct PrimSnapshot {
    idx: usize,
    local_id: u32,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
    volume: aurora_prim::VolumeParams,
    te: Option<Arc<aurora_prim::TextureEntry>>,
    media_url: String,
    gltf_asset: Option<Vec<(u8, Uuid)>>,
    gltf_overrides: Option<Arc<[(u8, PbrOverride)]>>,
    physics_flags: u32,
}

/// Pure helper computing batched [`TransformUpdate`]s for a linkset/group
/// (`translate`, `rotate_deg` around `pivot`, and `scale_factor` clamped to `[0.01, 64.0]`).
pub(super) fn compute_group_transforms(
    targets: &[GroupTransformTarget],
    translate: Option<[f32; 3]>,
    rotate_deg: Option<[f32; 3]>,
    pivot: Option<[f32; 3]>,
    scale_factor: Option<[f32; 3]>,
) -> Vec<(TransformUpdate, Vec3, Quat, Vec3)> {
    if targets.is_empty() {
        return Vec::new();
    }
    let default_pivot = targets.iter().find(|t| t.is_root).map(|t| t.position).unwrap_or_else(|| {
        let sum: Vec3 = targets.iter().map(|t| t.position).sum();
        sum / targets.len() as f32
    });
    let pivot_v = pivot.map(Vec3::from_array).unwrap_or(default_pivot);
    let trans_v = translate.map(Vec3::from_array).unwrap_or(Vec3::ZERO);
    let delta_rot = rotate_deg
        .map(|r| Quat::from_euler(EulerRot::XYZ, r[0].to_radians(), r[1].to_radians(), r[2].to_radians()))
        .unwrap_or(Quat::IDENTITY);
    let scale_f = scale_factor.map(Vec3::from_array).unwrap_or(Vec3::ONE);
    let has_scale = scale_factor.is_some();

    let clamp_scale = |s: Vec3| Vec3::new(s.x.clamp(0.01, 64.0), s.y.clamp(0.01, 64.0), s.z.clamp(0.01, 64.0));

    targets
        .iter()
        .map(|t| {
            let rel = t.position - pivot_v;
            let scaled_rel = if has_scale { rel * scale_f } else { rel };
            let new_pos = pivot_v + delta_rot * scaled_rel + trans_v;
            let new_rot = (delta_rot * t.rotation).normalize();
            let new_scale = if has_scale { clamp_scale(t.scale * scale_f) } else { t.scale };
            (
                TransformUpdate {
                    local_id: t.local_id,
                    position: Some(new_pos),
                    rotation: Some(new_rot),
                    scale: has_scale.then_some(new_scale),
                    linked: false,
                    uniform: false,
                },
                new_pos,
                new_rot,
                new_scale,
            )
        })
        .collect()
}

fn parse_alpha_mode(s: &str) -> Result<AlphaMode, McpError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "opaque" => Ok(AlphaMode::Opaque),
        "blend" => Ok(AlphaMode::Blend),
        "mask" => Ok(AlphaMode::Mask),
        other => Err(McpError::InvalidParams(format!(
            "unknown alpha_mode '{other}'; expected opaque|blend|mask"
        ))),
    }
}

fn parse_uuid_value(v: &Value, field: &str) -> Result<Uuid, McpError> {
    let s = v
        .as_str()
        .map(str::trim)
        .ok_or_else(|| McpError::InvalidParams(format!("'{field}' must be a UUID string")))?;
    Uuid::parse_str(s).map_err(|e| McpError::InvalidParams(format!("invalid UUID for '{field}': {e}")))
}

fn json_f32_3(v: Option<&Value>, key: &str) -> Result<Option<[f32; 3]>, McpError> {
    let Some(val) = v else { return Ok(None) };
    let arr = val
        .as_array()
        .ok_or_else(|| McpError::InvalidParams(format!("'{key}' must be [x, y, z]")))?;
    if arr.len() != 3 {
        return Err(McpError::InvalidParams(format!("'{key}' must have 3 numbers")));
    }
    Ok(Some([
        arr[0].as_f64().unwrap_or(0.0) as f32,
        arr[1].as_f64().unwrap_or(0.0) as f32,
        arr[2].as_f64().unwrap_or(0.0) as f32,
    ]))
}

fn json_f32_4(v: Option<&Value>, key: &str) -> Result<Option<[f32; 4]>, McpError> {
    let Some(val) = v else { return Ok(None) };
    let arr = val
        .as_array()
        .ok_or_else(|| McpError::InvalidParams(format!("'{key}' must be [x, y, z, w]")))?;
    if arr.len() != 4 {
        return Err(McpError::InvalidParams(format!("'{key}' must have 4 numbers")));
    }
    Ok(Some([
        arr[0].as_f64().unwrap_or(0.0) as f32,
        arr[1].as_f64().unwrap_or(0.0) as f32,
        arr[2].as_f64().unwrap_or(0.0) as f32,
        arr[3].as_f64().unwrap_or(1.0) as f32,
    ]))
}

impl App {
    pub(super) fn start_mcp_prim_duplicate(
        &mut self,
        local_ids: Vec<u32>,
        offset: [f32; 3],
        select_copy: bool,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let Some(handle) = self.world.main_region else {
            let _ = reply_tx.send(Err(McpError::NotInWorld));
            return;
        };
        if !self.world.rlva_can_build() {
            let _ = reply_tx.send(Err(McpError::PermissionDenied("RLVa prevents building here".into())));
            return;
        }
        for &lid in &local_ids {
            let idx = match self.resolve_prim_by_local_id(lid) {
                Ok((i, _)) => i,
                Err(e) => {
                    let _ = reply_tx.send(Err(e));
                    return;
                }
            };
            if let Err(e) = self.check_object_modify_permission(idx) {
                let _ = reply_tx.send(Err(e));
                return;
            }
            if let Some(obj) = self.world.objects.get(idx)
                && let Some(props) = self.build.props.get(&obj.full_id)
                && (props.owner_mask & perm::COPY) == 0
                && self.world.god_level < crate::world::inventory::actions::GOD_MAINTENANCE
            {
                let _ = reply_tx.send(Err(McpError::PermissionDenied(format!("prim {lid} lacks Copy permission"))));
                return;
            }
        }

        let known_ids: HashSet<u32> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.key.region == handle)
            .map(|(_, o)| o.key.local_id)
            .collect();

        let group_id = self.world.group_for_rez(false);
        const FLAGS_CREATE_SELECTED: u32 = 0x02;
        if let Err(e) = self.mcp_send(NetCommand::Build(BuildCmd::Duplicate {
            handle,
            local_ids: local_ids.clone(),
            offset: Vec3::from_array(offset),
            flags: if select_copy { FLAGS_CREATE_SELECTED } else { 0 },
            group_id,
        })) {
            let _ = reply_tx.send(Err(e));
            return;
        }

        self.mcp_bridge.pending_duplicates.push_back(PendingDuplicate {
            region: handle,
            known_ids,
            expected_count: local_ids.len(),
            last_polled: None,
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(300, 10_000)),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_prim_duplicate(&mut self) {
        let now = Instant::now();
        for mut p in std::mem::take(&mut self.mcp_bridge.pending_duplicates) {
            let expired = now.duration_since(p.started) >= p.deadline;
            if !expired && p.last_polled.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(50)) {
                self.mcp_bridge.pending_duplicates.push_back(p);
                continue;
            }
            p.last_polled = Some(now);
            let mut new_prims: Vec<(u32, Uuid, [f32; 3])> = self
                .world
                .objects
                .iter()
                .filter(|(_, o)| o.key.region == p.region && !o.is_avatar() && !p.known_ids.contains(&o.key.local_id))
                .map(|(_, o)| (o.key.local_id, o.full_id, [o.position.x, o.position.y, o.position.z]))
                .collect();
            if new_prims.len() >= p.expected_count || expired {
                new_prims.sort_by_key(|(lid, _, _)| *lid);
                let created: Vec<Value> = new_prims
                    .into_iter()
                    .map(|(local_id, object_uuid, position)| {
                        json!({
                            "local_id": local_id,
                            "object_uuid": object_uuid.to_string(),
                            "position": position,
                        })
                    })
                    .collect();
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "duplicated_count": created.len(),
                    "created": created,
                }))));
            } else {
                self.mcp_bridge.pending_duplicates.push_back(p);
            }
        }
    }

    pub(super) fn handle_mcp_prim_select(&mut self, action: &str, local_ids: Vec<u32>, extend: bool) -> Result<ViewerResponse, McpError> {
        if action.eq_ignore_ascii_case("deselect") {
            if local_ids.is_empty() {
                self.build.deselect_all(&self.world);
            } else {
                let mut keys = Vec::new();
                for &lid in &local_ids {
                    let (idx, _) = self.resolve_prim_by_local_id(lid)?;
                    if let Some(o) = self.world.objects.get(idx) {
                        keys.push(o.key);
                    }
                }
                self.build.deselect(&self.world, &keys);
            }
            self.flush_build();
        } else {
            let mut idxs = Vec::with_capacity(local_ids.len());
            for &lid in &local_ids {
                let (idx, _) = self.resolve_prim_by_local_id(lid)?;
                idxs.push(idx);
            }
            if !extend {
                self.build.deselect_all(&self.world);
            }
            self.build.select_many(&mut self.world, &self.settings.build, &idxs);
            self.flush_build();
        }

        let selected: Vec<u32> = self.build.selection.iter().map(|k| k.local_id).collect();
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "count": selected.len(),
            "selected_local_ids": selected,
        })))
    }

    pub(super) fn handle_mcp_linkset_transform(
        &mut self,
        root_local_id: Option<u32>,
        local_ids: Vec<u32>,
        translate: Option<[f32; 3]>,
        rotate_deg: Option<[f32; 3]>,
        pivot: Option<[f32; 3]>,
        scale_factor: Option<[f32; 3]>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };

        let mut target_idxs = Vec::new();
        if let Some(root_lid) = root_local_id {
            let (root_idx, _) = self.resolve_prim_by_local_id(root_lid)?;
            target_idxs.push(root_idx);
            if let Some(root_key) = self.world.objects.get(root_idx).map(|o| o.key) {
                for &ci in self.world.objects.children_of(&root_key) {
                    if !target_idxs.contains(&ci) {
                        target_idxs.push(ci);
                    }
                }
            }
        } else {
            for lid in &local_ids {
                let (idx, _) = self.resolve_prim_by_local_id(*lid)?;
                if !target_idxs.contains(&idx) {
                    target_idxs.push(idx);
                }
            }
        }

        let mut targets = Vec::with_capacity(target_idxs.len());
        for &idx in &target_idxs {
            self.check_object_modify_permission(idx)?;
            if let Some(o) = self.world.objects.get(idx) {
                targets.push(GroupTransformTarget {
                    local_id: o.key.local_id,
                    is_root: o.parent_id == 0,
                    position: o.position,
                    rotation: o.rotation,
                    scale: o.scale,
                });
            }
        }

        let computed = compute_group_transforms(&targets, translate, rotate_deg, pivot, scale_factor);
        let mut updates = Vec::with_capacity(computed.len());
        for (i, (upd, new_pos, new_rot, new_scale)) in computed.into_iter().enumerate() {
            updates.push(upd);
            if let Some(o) = self.world.objects.get_mut(target_idxs[i]) {
                o.position = new_pos;
                o.rotation = new_rot;
                o.scale = new_scale;
            }
        }

        self.mcp_send(NetCommand::Build(BuildCmd::Transform { handle, updates }))?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "transformed_count": target_idxs.len(),
            "local_ids": target_idxs.iter().filter_map(|&i| self.world.objects.get(i).map(|o| o.key.local_id)).collect::<Vec<_>>(),
        })))
    }

    pub(super) fn handle_mcp_build_transaction(&mut self, ops: Vec<Value>, dry_run: bool) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        if ops.is_empty() || ops.len() > 128 {
            return Err(McpError::InvalidParams("ops must contain 1..=128 operations".into()));
        }

        // Preflight & snapshot all touched local_ids before any mutation.
        let mut snapshots: HashMap<u32, PrimSnapshot> = HashMap::new();
        for (i, op) in ops.iter().enumerate() {
            let kind = op
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::InvalidParams(format!("op[{i}] missing string 'op'")))?;
            if !matches!(
                kind,
                "transform" | "shape" | "material" | "gltf_material" | "metadata" | "physics" | "prim_media"
            ) {
                return Err(McpError::InvalidParams(format!("op[{i}] unknown op '{kind}'")));
            }
            let local_id = op
                .get("local_id")
                .and_then(Value::as_u64)
                .map(|v| v as u32)
                .ok_or_else(|| McpError::InvalidParams(format!("op[{i}] missing 'local_id'")))?;
            let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
            self.check_object_modify_permission(idx)?;
            if !snapshots.contains_key(&local_id)
                && let Some(o) = self.world.objects.get(idx)
            {
                snapshots.insert(
                    local_id,
                    PrimSnapshot {
                        idx,
                        local_id,
                        position: o.position,
                        rotation: o.rotation,
                        scale: o.scale,
                        volume: o.volume,
                        te: o.te.clone(),
                        media_url: o.media_url.clone(),
                        gltf_asset: (!o.extra.render_materials.is_empty()).then(|| o.extra.render_materials.clone()),
                        gltf_overrides: self.world.gltf_overrides.get(&o.key).cloned(),
                        physics_flags: o.update_flags,
                    },
                );
            }
        }

        if dry_run {
            let mut ids: Vec<u32> = snapshots.keys().copied().collect();
            ids.sort_unstable();
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "dry_run": true,
                "validated_ops": ops.len(),
                "affected_local_ids": ids,
            })));
        }

        let mut results = Vec::with_capacity(ops.len());
        let mut applied_ids: Vec<u32> = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            let local_id = op["local_id"].as_u64().unwrap() as u32;
            let res = self.execute_build_tx_op(op, local_id);
            match res {
                Ok(v) => {
                    if !applied_ids.contains(&local_id) {
                        applied_ids.push(local_id);
                    }
                    results.push(v);
                }
                Err(err) => {
                    // Roll back previously modified prims in reverse order.
                    for rid in applied_ids.into_iter().rev() {
                        if let Some(snap) = snapshots.get(&rid) {
                            self.restore_prim_snapshot(handle, snap);
                        }
                    }
                    return Ok(ViewerResponse::Json(json!({
                        "ok": false,
                        "rolled_back": true,
                        "failed_op_index": i,
                        "error": err.to_string(),
                    })));
                }
            }
        }

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "committed_ops": results.len(),
            "results": results,
        })))
    }

    fn execute_build_tx_op(&mut self, op: &Value, local_id: u32) -> Result<Value, McpError> {
        let kind = op["op"].as_str().unwrap_or("");
        let resp = match kind {
            "transform" => {
                let spec = PrimTransformSpec {
                    local_id,
                    position: json_f32_3(op.get("position"), "position")?,
                    rotation_quat: json_f32_4(op.get("rotation_quat"), "rotation_quat")?,
                    rotation_euler_deg: json_f32_3(op.get("rotation_euler_deg"), "rotation_euler_deg")?,
                    scale: json_f32_3(op.get("scale"), "scale")?,
                    name: op.get("name").and_then(Value::as_str).map(str::to_owned),
                    description: op.get("description").and_then(Value::as_str).map(str::to_owned),
                    root_only: op.get("root_only").and_then(Value::as_bool).unwrap_or(false),
                };
                self.handle_mcp_prim_transform(vec![spec])?
            }
            "shape" => {
                let pair = |k: &str| -> Option<[f32; 2]> {
                    let a = op.get(k)?.as_array()?;
                    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32])
                };
                let color = json_f32_4(op.get("color"), "color")?;
                let tex = op.get("texture_id").map(|v| parse_uuid_value(v, "texture_id")).transpose()?;
                self.handle_mcp_prim_set_shape(
                    local_id,
                    pair("path_cut"),
                    pair("profile_cut"),
                    op.get("hollow").and_then(Value::as_f64).map(|v| v as f32),
                    pair("twist"),
                    color,
                    tex,
                )?
            }
            "material" => {
                let pair = |k: &str| -> Option<[f32; 2]> {
                    let a = op.get(k)?.as_array()?;
                    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32])
                };
                self.handle_mcp_prim_set_material(
                    local_id,
                    op.get("face").and_then(Value::as_i64).map(|v| v as i32),
                    op.get("texture_id").map(|v| parse_uuid_value(v, "texture_id")).transpose()?,
                    op.get("material_id").map(|v| parse_uuid_value(v, "material_id")).transpose()?,
                    json_f32_4(op.get("color"), "color")?,
                    pair("repeats"),
                    pair("offsets"),
                    op.get("rotation_rad").and_then(Value::as_f64).map(|v| v as f32),
                    op.get("glow").and_then(Value::as_f64).map(|v| v as f32),
                    op.get("fullbright").and_then(Value::as_bool),
                    op.get("alpha_mode").and_then(Value::as_u64).map(|v| v as u8),
                    op.get("bump_shiny").and_then(Value::as_u64).map(|v| v as u8),
                )?
            }
            "gltf_material" => self.handle_mcp_prim_set_gltf_material(
                local_id,
                op.get("face").and_then(Value::as_i64).map(|v| v as i32),
                op.get("asset_id")
                    .or_else(|| op.get("material_id"))
                    .map(|v| parse_uuid_value(v, "asset_id"))
                    .transpose()?,
                op.get("clear_override").and_then(Value::as_bool).unwrap_or(false),
                op,
            )?,
            "metadata" => self.handle_mcp_prim_set_metadata(
                local_id,
                op.get("name").and_then(Value::as_str),
                op.get("description").and_then(Value::as_str),
            )?,
            "physics" => self.handle_mcp_prim_set_physics_properties(
                local_id,
                op.get("physics").and_then(Value::as_bool),
                op.get("temporary").and_then(Value::as_bool),
                op.get("phantom").and_then(Value::as_bool),
                op.get("shape_type").and_then(Value::as_str),
                op.get("density").and_then(Value::as_f64).map(|v| v as f32),
                op.get("friction").and_then(Value::as_f64).map(|v| v as f32),
                op.get("restitution").and_then(Value::as_f64).map(|v| v as f32),
                op.get("gravity_multiplier").and_then(Value::as_f64).map(|v| v as f32),
                op.get("animated_mesh").and_then(Value::as_bool),
            )?,
            "prim_media" => self.handle_mcp_prim_set_media(
                local_id,
                op.get("face").and_then(Value::as_u64).unwrap_or(0) as u8,
                op.get("clear").and_then(Value::as_bool).unwrap_or(false),
                op,
            )?,
            other => return Err(McpError::InvalidParams(format!("unknown tx op '{other}'"))),
        };
        Ok(match resp {
            ViewerResponse::Json(v) => v,
            _ => json!({ "ok": true, "local_id": local_id }),
        })
    }

    fn restore_prim_snapshot(&mut self, handle: RegionHandle, snap: &PrimSnapshot) {
        if let Some(o) = self.world.objects.get_mut(snap.idx) {
            o.position = snap.position;
            o.rotation = snap.rotation;
            o.scale = snap.scale;
            o.update_flags = snap.physics_flags;
            if let Some(te) = &snap.te {
                o.te = Some(Arc::clone(te));
                o.material_dirty = true;
            }
            match &snap.gltf_overrides {
                Some(ov) => {
                    self.world.gltf_overrides.insert(o.key, Arc::clone(ov));
                }
                None => {
                    self.world.gltf_overrides.remove(&o.key);
                }
            }
            if let Some(mats) = &snap.gltf_asset {
                o.extra.render_materials = mats.clone();
            } else {
                o.extra.render_materials.clear();
            }
        }
        self.send(NetCommand::Build(BuildCmd::Transform {
            handle,
            updates: vec![TransformUpdate {
                local_id: snap.local_id,
                position: Some(snap.position),
                rotation: Some(snap.rotation),
                scale: Some(snap.scale),
                linked: false,
                uniform: false,
            }],
        }));
        self.build.set_shape(&mut self.world, snap.idx, snap.volume);
        if let Some(te) = &snap.te {
            let num_faces = self.build.num_faces(&self.world, snap.idx);
            let packed = aurora_prim::te::pack_texture_entry(te, num_faces);
            self.send(NetCommand::Build(BuildCmd::SetTextures {
                handle,
                local_id: snap.local_id,
                media_url: snap.media_url.clone(),
                texture_entry: packed,
            }));
        }
        self.send(NetCommand::Build(BuildCmd::SetFlags {
            handle,
            local_id: snap.local_id,
            physics: (snap.physics_flags & crate::build::edits::FLAGS_USE_PHYSICS) != 0,
            temporary: (snap.physics_flags & crate::build::edits::FLAGS_TEMPORARY_ON_REZ) != 0,
            phantom: (snap.physics_flags & crate::build::edits::FLAGS_PHANTOM) != 0,
        }));
        self.flush_build();
    }

    fn resolve_script_item(
        &self,
        local_id: Option<u32>,
        object_uuid: Option<Uuid>,
        item_id: Option<Uuid>,
        script_name: Option<&str>,
    ) -> Result<(RegionHandle, Uuid, Uuid), McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let idx = if let Some(lid) = local_id {
            self.resolve_prim_by_local_id(lid)?.0
        } else if let Some(uid) = object_uuid {
            self.world
                .objects
                .index_of_uuid(&uid)
                .ok_or_else(|| McpError::InvalidParams(format!("Object {uid} not found in region")))?
        } else {
            return Err(McpError::InvalidParams("provide 'local_id' or 'object_uuid'".into()));
        };
        self.check_object_modify_permission(idx)?;
        let Some(obj) = self.world.objects.get(idx) else {
            return Err(McpError::InvalidParams("Object not found".into()));
        };
        let obj_uuid = obj.full_id;

        if let Some(id) = item_id {
            return Ok((handle, obj_uuid, id));
        }
        let target_name = script_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| McpError::InvalidParams("provide 'item_id' or 'script_name'".into()))?;
        let Some(task_inv) = self.build.contents.objects.get(&obj_uuid) else {
            return Err(McpError::InvalidParams(
                "object inventory not loaded yet: call `object_contents` or `lsl_inspect_object_scripts` first, or pass `item_id`".into(),
            ));
        };
        let matches: Vec<&aurora_net::build::TaskItem> = task_inv
            .items
            .iter()
            .filter(|it| !it.is_folder && it.asset_type == 10 && it.name == target_name)
            .collect();
        match matches[..] {
            [one] => Ok((handle, obj_uuid, one.item_id)),
            [] => Err(McpError::InvalidParams(format!(
                "no script named '{target_name}' found in object {obj_uuid}"
            ))),
            _ => Err(McpError::InvalidParams(format!(
                "multiple scripts named '{target_name}' in object {obj_uuid}; pass explicit 'item_id'"
            ))),
        }
    }

    pub(super) fn handle_mcp_script_reset(
        &mut self,
        local_id: Option<u32>,
        object_uuid: Option<Uuid>,
        item_id: Option<Uuid>,
        script_name: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        let (handle, obj_uuid, script_id) = self.resolve_script_item(local_id, object_uuid, item_id, script_name)?;
        self.mcp_send(NetCommand::Build(BuildCmd::ScriptReset {
            handle,
            object_id: obj_uuid,
            item_id: script_id,
        }))?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "reset": true,
            "object_uuid": obj_uuid.to_string(),
            "item_id": script_id.to_string(),
        })))
    }

    pub(super) fn handle_mcp_script_set_running(
        &mut self,
        local_id: Option<u32>,
        object_uuid: Option<Uuid>,
        item_id: Option<Uuid>,
        script_name: Option<&str>,
        running: bool,
    ) -> Result<ViewerResponse, McpError> {
        let (handle, obj_uuid, script_id) = self.resolve_script_item(local_id, object_uuid, item_id, script_name)?;
        self.mcp_send(NetCommand::Build(BuildCmd::SetScriptRunning {
            handle,
            object_id: obj_uuid,
            item_id: script_id,
            running,
        }))?;
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "object_uuid": obj_uuid.to_string(),
            "item_id": script_id.to_string(),
            "running": running,
        })))
    }

    /// Resolve an inventory item (or direct asset UUID) before applying a texture or material
    /// to a prim, enforcing attachment RLVa locks and `BuildTool::admit_item`.
    pub(super) fn resolve_and_admit_prim_asset(
        &mut self,
        idx: usize,
        id: Uuid,
        expected_asset_type: i32,
        kind_label: &str,
    ) -> Result<Uuid, McpError> {
        if crate::build::contents_drop::attachment_locked(&self.world, idx) {
            return Err(McpError::PermissionDenied("RLVa locks this attachment".into()));
        }
        let target_id = crate::world::inventory::actions::original(&self.world.inventory, id).unwrap_or(id);
        if let Some(item) = self.world.inventory.items.get(&target_id).cloned() {
            if item.asset_type != expected_asset_type {
                return Err(McpError::InvalidParams(format!(
                    "inventory item '{}' is not a {kind_label} (asset_type {})",
                    item.name, item.asset_type
                )));
            }
            self.build
                .admit_item(&mut self.world, target_id, &[idx])
                .map_err(|m| McpError::PermissionDenied(m.to_owned()))?;
            Ok(item.asset_id)
        } else {
            Ok(id)
        }
    }

    pub(super) fn handle_mcp_prim_set_gltf_material(
        &mut self,
        local_id: u32,
        face: Option<i32>,
        asset_id: Option<Uuid>,
        clear_override: bool,
        spec: &Value,
    ) -> Result<ViewerResponse, McpError> {
        if self.world.main_region.is_none() {
            return Err(McpError::NotInWorld);
        }
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        self.check_object_modify_permission(idx)?;
        if !self.world.rlva_can_edit(idx) {
            return Err(McpError::PermissionDenied("RLVa prevents editing this object".into()));
        }
        let num_faces = self.build.num_faces(&self.world, idx).max(1);
        let faces_here = num_faces.min(aurora_prim::te::MAX_TES);
        let target_faces: Vec<u8> = match face {
            Some(f) if f >= 0 && (f as usize) < faces_here => vec![f as u8],
            Some(f) if f >= 0 => {
                return Err(McpError::InvalidParams(format!("face {f} out of range (0..{})", faces_here - 1)));
            }
            _ => (0..faces_here as u8).collect(),
        };

        let resolved_asset = asset_id
            .map(|mat| self.resolve_and_admit_prim_asset(idx, mat, 57, "glTF material"))
            .transpose()?;

        let alpha_mode = spec.get("alpha_mode").and_then(Value::as_str).map(parse_alpha_mode).transpose()?;
        let base_color = json_f32_4(spec.get("base_color_factor"), "base_color_factor")?;
        let emissive = json_f32_3(spec.get("emissive_factor"), "emissive_factor")?;
        let metallic = spec
            .get("metallic_factor")
            .and_then(Value::as_f64)
            .map(|v| (v as f32).clamp(0.0, 1.0));
        let roughness = spec
            .get("roughness_factor")
            .and_then(Value::as_f64)
            .map(|v| (v as f32).clamp(0.0, 1.0));
        let alpha_cutoff = spec.get("alpha_cutoff").and_then(Value::as_f64).map(|v| (v as f32).clamp(0.0, 1.0));
        let double_sided = spec.get("double_sided").and_then(Value::as_bool);

        let raw_tex_slots = [
            spec.get("base_color_texture")
                .map(|v| parse_uuid_value(v, "base_color_texture"))
                .transpose()?,
            spec.get("normal_texture")
                .map(|v| parse_uuid_value(v, "normal_texture"))
                .transpose()?,
            spec.get("metallic_roughness_texture")
                .map(|v| parse_uuid_value(v, "metallic_roughness_texture"))
                .transpose()?,
            spec.get("emissive_texture")
                .map(|v| parse_uuid_value(v, "emissive_texture"))
                .transpose()?,
        ];
        let tex_slots = [
            raw_tex_slots[0]
                .map(|u| self.resolve_and_admit_prim_asset(idx, u, 0, "texture"))
                .transpose()?,
            raw_tex_slots[1]
                .map(|u| self.resolve_and_admit_prim_asset(idx, u, 0, "texture"))
                .transpose()?,
            raw_tex_slots[2]
                .map(|u| self.resolve_and_admit_prim_asset(idx, u, 0, "texture"))
                .transpose()?,
            raw_tex_slots[3]
                .map(|u| self.resolve_and_admit_prim_asset(idx, u, 0, "texture"))
                .transpose()?,
        ];

        let mut updates = Vec::with_capacity(target_faces.len());
        for f in &target_faces {
            let over = if clear_override {
                None
            } else {
                let mut ov = crate::build::BuildTool::gltf_override(&self.world, idx, *f);
                if let Some(bc) = base_color {
                    ov.base_color_factor = Some(bc);
                }
                if let Some(ec) = emissive {
                    ov.emissive_factor = Some(ec);
                }
                if let Some(m) = metallic {
                    ov.metallic_factor = Some(m);
                }
                if let Some(r) = roughness {
                    ov.roughness_factor = Some(r);
                }
                if let Some(am) = alpha_mode {
                    ov.alpha_mode = Some(am);
                }
                if let Some(ac) = alpha_cutoff {
                    ov.alpha_cutoff = Some(ac);
                }
                if let Some(ds) = double_sided {
                    ov.double_sided = Some(ds);
                }
                for (slot_i, tex) in tex_slots.iter().enumerate() {
                    if let Some(u) = tex {
                        ov.textures[slot_i] = *u;
                    }
                }
                if let Some(arr) = spec.get("transforms").and_then(Value::as_array) {
                    for (slot_i, entry) in arr.iter().take(4).enumerate() {
                        if entry.is_object() {
                            let off = entry.get("offset").and_then(Value::as_array).map(|a| {
                                [
                                    a.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
                                    a.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
                                ]
                            });
                            let sc = entry.get("scale").and_then(Value::as_array).map(|a| {
                                [
                                    a.first().and_then(Value::as_f64).unwrap_or(1.0) as f32,
                                    a.get(1).and_then(Value::as_f64).unwrap_or(1.0) as f32,
                                ]
                            });
                            let rot = entry.get("rotation").and_then(Value::as_f64).map(|v| v as f32);
                            let cur = &ov.transforms[slot_i];
                            ov.transforms[slot_i] = TextureTransform {
                                offset: off.unwrap_or(cur.offset),
                                scale: sc.unwrap_or(cur.scale),
                                rotation: rot.unwrap_or(cur.rotation),
                            };
                        }
                    }
                }
                Some(ov)
            };
            updates.push(GltfUpdate {
                object: idx,
                face: *f,
                asset: resolved_asset,
                over,
            });
        }

        self.build.gltf_update(&mut self.world, updates);
        self.flush_build();
        let ids = crate::world::inventory::asset_ids::AssetIds::of(&self.world);
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "faces": target_faces,
            "asset_id": resolved_asset.map(|u| ids.id(u).to_string()),
        })))
    }

    pub(super) fn handle_mcp_prim_set_media(
        &mut self,
        local_id: u32,
        face: u8,
        clear: bool,
        spec: &Value,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        self.check_object_modify_permission(idx)?;
        if !self.world.rlva_can_edit(idx) {
            return Err(McpError::PermissionDenied("RLVa prevents editing this object".into()));
        }
        let num_faces = self.build.num_faces(&self.world, idx).max(1);
        if (face as usize) >= num_faces {
            return Err(McpError::InvalidParams(format!(
                "face {face} does not exist (0..{})",
                num_faces - 1
            )));
        }
        let obj_uuid = self.world.objects.get(idx).unwrap().full_id;
        let mut faces: Vec<Option<MediaEntry>> = self
            .build
            .object_media(&self.media, &obj_uuid)
            .map(|d| d.faces.clone())
            .unwrap_or_default();
        if faces.len() < num_faces {
            faces.resize_with(num_faces, || None);
        }

        if clear {
            faces[face as usize] = None;
        } else {
            let mut entry = faces[face as usize].clone().unwrap_or_default();
            if let Some(u) = spec.get("url").or_else(|| spec.get("current_url")).and_then(Value::as_str) {
                let valid = super::validate_mcp_url(u, &["http://", "https://"], "current_url")?;
                entry.current_url = valid.clone();
                if entry.home_url.is_empty() {
                    entry.home_url = valid;
                }
            }
            if let Some(h) = spec.get("home_url").and_then(Value::as_str) {
                entry.home_url = super::validate_mcp_url(h, &["http://", "https://"], "home_url")?;
            }
            if let Some(v) = spec.get("auto_play").and_then(Value::as_bool) {
                entry.auto_play = v;
            }
            if let Some(v) = spec.get("auto_loop").and_then(Value::as_bool) {
                entry.auto_loop = v;
            }
            if let Some(v) = spec.get("auto_scale").and_then(Value::as_bool) {
                entry.auto_scale = v;
            }
            if let Some(v) = spec.get("auto_zoom").and_then(Value::as_bool) {
                entry.auto_zoom = v;
            }
            if let Some(v) = spec.get("first_click_interact").and_then(Value::as_bool) {
                entry.first_click_interact = v;
            }
            if let Some(v) = spec.get("width_pixels").and_then(Value::as_u64) {
                entry.width_pixels = v.min(2048) as u16;
            }
            if let Some(v) = spec.get("height_pixels").and_then(Value::as_u64) {
                entry.height_pixels = v.min(2048) as u16;
            }
            if let Some(v) = spec.get("controls").and_then(Value::as_u64) {
                entry.controls = v as u8;
            }
            if let Some(v) = spec.get("whitelist_enable").and_then(Value::as_bool) {
                entry.whitelist_enable = v;
            }
            if let Some(arr) = spec.get("whitelist").and_then(Value::as_array) {
                entry.whitelist = arr.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            faces[face as usize] = Some(entry);
        }

        // Update MF_HAS_MEDIA bit on the face's TextureEntry + dispatch ObjectMedia UPDATE.
        if let Some(obj) = self.world.objects.get_mut(idx) {
            let mut te = obj.te.as_deref().cloned().unwrap_or_default();
            if let Some(tf) = te.faces.get_mut(face as usize) {
                if clear {
                    tf.media_flags &= !crate::build::materials::MF_HAS_MEDIA;
                } else {
                    tf.media_flags |= crate::build::materials::MF_HAS_MEDIA;
                }
            }
            let packed = aurora_prim::te::pack_texture_entry(&te, num_faces);
            let media_url = obj.media_url.clone();
            obj.te = Some(Arc::new(te));
            obj.material_dirty = true;
            self.send(NetCommand::Build(BuildCmd::SetTextures {
                handle,
                local_id,
                media_url,
                texture_entry: packed,
            }));
        }

        self.build.media_update(&self.world, idx, &faces);
        self.flush_build();
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "face": face,
            "cleared": clear,
        })))
    }

    pub(super) fn handle_mcp_prim_set_sculpt(
        &mut self,
        local_id: u32,
        sculpt_texture: Option<Uuid>,
        sculpt_type: Option<&str>,
        invert: Option<bool>,
        mirror: Option<bool>,
        clear_sculpt: bool,
        flexible: Option<Value>,
        light: Option<Value>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        self.check_object_modify_permission(idx)?;
        if !self.world.rlva_can_edit(idx) {
            return Err(McpError::PermissionDenied("RLVa prevents editing this object".into()));
        }

        if clear_sculpt {
            if let Some(o) = self.world.objects.get_mut(idx) {
                o.extra.sculpt = None;
                o.volume.sculpt = None;
                o.shape_dirty = true;
            }
            self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                handle,
                local_id,
                param_type: PARAMS_SCULPT,
                in_use: false,
                data: Vec::new(),
            }))?;
        } else if sculpt_texture.is_some() || sculpt_type.is_some() || invert.is_some() || mirror.is_some() {
            let resolved_sculpt_texture = sculpt_texture
                .map(|tex| self.resolve_and_admit_prim_asset(idx, tex, 0, "texture"))
                .transpose()?;
            let mut sp = self.world.objects.get(idx).and_then(|o| o.extra.sculpt).unwrap_or(SculptParams {
                texture: Uuid::nil(),
                sculpt_type: 1,
            });
            if let Some(tex) = resolved_sculpt_texture {
                sp.texture = tex;
            }
            if let Some(st) = sculpt_type {
                let kind = match st.trim().to_ascii_lowercase().as_str() {
                    "sphere" => 1,
                    "torus" => 2,
                    "plane" => 3,
                    "cylinder" => 4,
                    "mesh" => 5,
                    other => return Err(McpError::InvalidParams(format!("unknown sculpt_type '{other}'"))),
                };
                sp.sculpt_type = (sp.sculpt_type & !0x07) | kind;
            }
            if let Some(inv) = invert {
                if inv {
                    sp.sculpt_type |= LL_SCULPT_FLAG_INVERT;
                } else {
                    sp.sculpt_type &= !LL_SCULPT_FLAG_INVERT;
                }
            }
            if let Some(mir) = mirror {
                if mir {
                    sp.sculpt_type |= LL_SCULPT_FLAG_MIRROR;
                } else {
                    sp.sculpt_type &= !LL_SCULPT_FLAG_MIRROR;
                }
            }
            if let Some(o) = self.world.objects.get_mut(idx) {
                o.extra.sculpt = Some(sp);
                o.volume.sculpt = Some(sp);
                o.shape_dirty = true;
            }
            self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                handle,
                local_id,
                param_type: PARAMS_SCULPT,
                in_use: true,
                data: pack_sculpt(&sp),
            }))?;
        }

        if let Some(flex) = flexible {
            let enabled = flex.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                if let Some(o) = self.world.objects.get_mut(idx) {
                    o.extra.flexible = None;
                }
                self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                    handle,
                    local_id,
                    param_type: PARAMS_FLEXIBLE,
                    in_use: false,
                    data: Vec::new(),
                }))?;
            } else {
                let cur = self.world.objects.get(idx).and_then(|o| o.extra.flexible);
                let fp = FlexibleParams {
                    softness: flex
                        .get("softness")
                        .and_then(Value::as_u64)
                        .unwrap_or(cur.map_or(2, |c| c.softness as u64)) as u8,
                    tension: flex
                        .get("tension")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(1.0, |c| c.tension as f64)) as f32,
                    air_friction: flex
                        .get("air_friction")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(2.0, |c| c.air_friction as f64)) as f32,
                    gravity: flex
                        .get("gravity")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(0.3, |c| c.gravity as f64)) as f32,
                    wind_sensitivity: flex
                        .get("wind_sensitivity")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(0.0, |c| c.wind_sensitivity as f64)) as f32,
                    user_force: json_f32_3(flex.get("user_force"), "user_force")?
                        .map(Vec3::from_array)
                        .unwrap_or(cur.map_or(Vec3::ZERO, |c| c.user_force)),
                };
                if let Some(o) = self.world.objects.get_mut(idx) {
                    o.extra.flexible = Some(fp);
                }
                self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                    handle,
                    local_id,
                    param_type: PARAMS_FLEXIBLE,
                    in_use: true,
                    data: pack_flexible(&fp),
                }))?;
            }
        }

        if let Some(lt) = light {
            let enabled = lt.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                if let Some(o) = self.world.objects.get_mut(idx) {
                    o.extra.light = None;
                }
                self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                    handle,
                    local_id,
                    param_type: PARAMS_LIGHT,
                    in_use: false,
                    data: Vec::new(),
                }))?;
            } else {
                let cur = self.world.objects.get(idx).and_then(|o| o.extra.light);
                let lp = LightParams {
                    color: json_f32_4(lt.get("color"), "color")?.unwrap_or(cur.map_or([1.0, 1.0, 1.0, 1.0], |c| c.color)),
                    radius: lt
                        .get("radius")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(10.0, |c| c.radius as f64)) as f32,
                    cutoff: lt
                        .get("cutoff")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(0.0, |c| c.cutoff as f64)) as f32,
                    falloff: lt
                        .get("falloff")
                        .and_then(Value::as_f64)
                        .unwrap_or(cur.map_or(0.75, |c| c.falloff as f64)) as f32,
                };
                if let Some(o) = self.world.objects.get_mut(idx) {
                    o.extra.light = Some(lp);
                }
                self.mcp_send(NetCommand::Build(BuildCmd::SetExtraParam {
                    handle,
                    local_id,
                    param_type: PARAMS_LIGHT,
                    in_use: true,
                    data: pack_light(&lp),
                }))?;
            }
        }

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
        })))
    }
}
