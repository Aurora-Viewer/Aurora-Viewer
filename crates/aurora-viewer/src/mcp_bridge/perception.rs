//! Phase 5 (C3) MCP perception handlers: `raycast`, `object_inspect`,
//! `region_info`, `map_lookup`, `search_people`, `search_places`,
//! `avatar_profile`, and `media_info`.

use super::*;
use crate::world::search as wsearch;
use aurora_net::RegionHandle;
use aurora_net::build::perm;

/// Object property / cost / media fetch awaiting reply for `object_inspect`.
pub(super) struct PendingObjectInspect {
    local_id: u32,
    object_uuid: Uuid,
    root_uuid: Uuid,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// World-map region lookup awaiting `MapBlocks` arrival.
pub(super) struct PendingMapLookup {
    name: Option<String>,
    coords: Option<(u16, u16)>,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// People/Places directory search or UUID-to-name resolution awaiting reply.
pub(super) struct PendingSearch {
    tab: wsearch::Tab,
    query: Option<String>,
    resolve_uuids: Vec<Uuid>,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Avatar profile request awaiting `Profiles::get(&agent_id)`.
pub(super) struct PendingProfile {
    agent_id: Uuid,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Prim/parcel media info request awaiting `ObjectMedia` fetch.
pub(super) struct PendingMediaInfo {
    local_id: Option<u32>,
    object_uuid: Option<Uuid>,
    started: Instant,
    deadline: Duration,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Pure oriented-bounding-box ray intersection helper `(distance, world_normal)`.
/// Works headless and in `DemoSim` without requiring GPU pick buffers.
pub(super) fn ray_box_hit(origin: Vec3, dir: Vec3, center: Vec3, rot: Quat, half: Vec3) -> Option<(f32, Vec3)> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let inv_rot = rot.inverse();
    let lo = inv_rot * (origin - center);
    let ld = inv_rot * dir;
    let h = half.max(Vec3::splat(0.02));

    let mut t_min = 0.0_f32;
    let mut t_max = f32::INFINITY;
    let mut hit_normal_local = Vec3::Z;

    for axis in 0..3 {
        let d = ld[axis];
        let o = lo[axis];
        let bound = h[axis];
        if d.abs() < 1e-6 {
            if o < -bound || o > bound {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let mut t1 = (-bound - o) * inv;
            let mut t2 = (bound - o) * inv;
            let mut n = Vec3::ZERO;
            n[axis] = -1.0;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
                n[axis] = 1.0;
            }
            if t1 > t_min {
                t_min = t1;
                hit_normal_local = n;
            }
            t_max = t_max.min(t2);
            if t_max < t_min {
                return None;
            }
        }
    }

    if t_min.is_finite() && t_min >= 0.0 {
        let world_normal = (rot * hit_normal_local).normalize_or(Vec3::Z);
        Some((t_min, world_normal))
    } else {
        None
    }
}

fn perm_summary(m: u32) -> Value {
    json!({
        "copy": (m & perm::COPY) != 0,
        "modify": (m & perm::MODIFY) != 0,
        "transfer": (m & perm::TRANSFER) != 0,
        "move": (m & perm::MOVE) != 0,
        "raw": m,
    })
}

fn search_val_to_json(v: &wsearch::Value) -> Value {
    match v {
        wsearch::Value::None => Value::Null,
        wsearch::Value::Int(i) => json!(i),
        wsearch::Value::Real(r) => json!(r),
        wsearch::Value::Text(s) => json!(s),
        wsearch::Value::Dated { text, unix } => json!({ "text": text, "unix": unix }),
    }
}

fn search_status_str(st: &wsearch::Status) -> &'static str {
    match st {
        wsearch::Status::Idle => "idle",
        wsearch::Status::Searching => "searching",
        wsearch::Status::Found => "found",
        wsearch::Status::NotFound(_) => "not_found",
        wsearch::Status::TooShort => "too_short",
        wsearch::Status::Banned => "banned",
        wsearch::Status::Disabled => "disabled",
        wsearch::Status::NoDateOffset => "no_date_offset",
        wsearch::Status::NoCategory => "no_category",
        wsearch::Status::NoQuery => "no_query",
        wsearch::Status::NoContent => "no_content",
    }
}

fn serialize_region_summary(r: &crate::world::Region, agent_pos: Option<[f32; 2]>, world: &World) -> Value {
    let h = r.handle;
    let gx = ((h >> 32) / 256) as u32;
    let gy = ((h & 0xffff_ffff) / 256) as u32;
    let off = world.region_offset(h).unwrap_or(Vec3::ZERO);
    let terrain_height = agent_pos.map(|[ax, ay]| {
        let lx = (ax - off.x).clamp(0.0, (r.heightmap.size_x.saturating_sub(1)) as f32);
        let ly = (ay - off.y).clamp(0.0, (r.heightmap.size_y.saturating_sub(1)) as f32);
        r.heightmap.sample(lx, ly)
    });
    let (region_id, size_x, size_y, water_height, sim_access, region_flags, owner_id, is_estate_manager, product_name) = match &r.info {
        Some(info) => (
            Some(info.region_id.to_string()),
            info.size_x,
            info.size_y,
            info.water_height,
            info.sim_access,
            info.region_flags,
            Some(info.owner.to_string()),
            info.is_estate_manager,
            info.product_name.clone(),
        ),
        None => (None, r.heightmap.size_x, r.heightmap.size_y, 20.0, 0, 0, None, false, String::new()),
    };
    json!({
        "name": r.name,
        "handle": h,
        "region_id": region_id,
        "grid_x": gx,
        "grid_y": gy,
        "size_x": size_x,
        "size_y": size_y,
        "water_height": water_height,
        "sim_access": sim_access,
        "access_label": crate::world::worldmap::access_label(sim_access),
        "region_flags": region_flags,
        "product_name": product_name,
        "owner_id": owner_id,
        "is_estate_manager": is_estate_manager,
        "max_tasks": r.max_tasks,
        "lua_scripts": r.lua_scripts,
        "max_link_children": r.max_link_children,
        "terrain_height": terrain_height,
    })
}

/// Pure object inspection snapshot builder (callable from `object_inspect` & tests).
pub(super) fn serialize_object_inspect(
    world: &World,
    build: &crate::build::BuildTool,
    media: &crate::media::MediaManager,
    idx: usize,
) -> Option<Value> {
    let obj = world.objects.get(idx)?;
    let ids = crate::world::inventory::asset_ids::AssetIds::of(world);
    let props = build.props.get(&obj.full_id);
    let root_id = if obj.parent_id == 0 {
        obj.full_id
    } else {
        world
            .objects
            .index_of(&crate::world::objects::ObjKey {
                region: obj.key.region,
                local_id: obj.parent_id,
            })
            .and_then(|ri| world.objects.get(ri))
            .map(|r| r.full_id)
            .unwrap_or(obj.full_id)
    };
    let children_local_ids: Vec<u32> = world
        .objects
        .children_of(&obj.key)
        .iter()
        .filter_map(|&ci| world.objects.get(ci).map(|c| c.key.local_id))
        .collect();
    let land_impact = build.costs.objects.get(&root_id).map(|c| c.linked_set);
    let phys = build.physics.get(&obj.key.local_id);
    let obj_media = build.object_media(media, &obj.full_id);
    let num_faces = build.num_faces(world, idx);
    let te_faces = obj.te.as_ref().map(|t| t.faces.as_slice()).unwrap_or(&[]);
    let face_count = num_faces.max(te_faces.len()).min(aurora_prim::te::MAX_TES);
    let mut faces = Vec::with_capacity(face_count);
    for f in 0..face_count {
        let tf = te_faces.get(f).copied().unwrap_or_default();
        let gltf_asset = crate::build::BuildTool::gltf_asset(world, idx, f as u8);
        let gltf_ov = crate::build::BuildTool::gltf_override(world, idx, f as u8);
        let has_override = world
            .gltf_overrides
            .get(&obj.key)
            .is_some_and(|list| list.iter().any(|(face_i, _)| *face_i == f as u8));
        faces.push(json!({
            "face": f,
            "texture_id": ids.id(tf.texture).to_string(),
            "color": tf.color,
            "repeats": [tf.scale_s, tf.scale_t],
            "offsets": [tf.offset_s, tf.offset_t],
            "rotation_rad": tf.rotation,
            "glow": tf.glow,
            "fullbright": tf.fullbright(),
            "shiny": tf.shiny(),
            "bump": tf.bump(),
            "planar": tf.planar(),
            "has_media": tf.has_media(),
            "legacy_material_id": ids.id(tf.material_id).to_string(),
            "gltf_material_id": gltf_asset.map(|id| ids.id(id).to_string()),
            "gltf_override": has_override.then(|| json!({
                "base_color_factor": gltf_ov.base_color_factor,
                "emissive_factor": gltf_ov.emissive_factor,
                "metallic_factor": gltf_ov.metallic_factor,
                "roughness_factor": gltf_ov.roughness_factor,
                "double_sided": gltf_ov.double_sided,
                "alpha_cutoff": gltf_ov.alpha_cutoff,
                "textures": gltf_ov.textures.map(|u| ids.id(u).to_string()),
            })),
        }));
    }

    let (rx, ry, rz) = obj.rotation.to_euler(EulerRot::XYZ);
    let permissions = props.map(|p| {
        json!({
            "base": perm_summary(p.base_mask),
            "owner": perm_summary(p.owner_mask),
            "group": perm_summary(p.group_mask),
            "everyone": perm_summary(p.everyone_mask),
            "next_owner": perm_summary(p.next_owner_mask),
        })
    });
    let owner_id = if !obj.owner_id.is_nil() {
        obj.owner_id
    } else {
        props.map_or(Uuid::nil(), |p| p.owner_id)
    };

    Some(json!({
        "ok": true,
        "local_id": obj.key.local_id,
        "object_uuid": obj.full_id.to_string(),
        "parent_id": obj.parent_id,
        "is_root": obj.parent_id == 0,
        "children_local_ids": children_local_ids,
        "name": props.map(|p| p.name.clone()).unwrap_or_default(),
        "description": props.map(|p| p.description.clone()).unwrap_or_default(),
        "touch_name": props.map(|p| p.touch_name.clone()).unwrap_or_default(),
        "sit_name": props.map(|p| p.sit_name.clone()).unwrap_or_default(),
        "owner_id": owner_id.to_string(),
        "creator_id": props.map(|p| p.creator_id.to_string()),
        "group_id": props.map(|p| p.group_id.to_string()),
        "last_owner_id": props.map(|p| p.last_owner_id.to_string()),
        "creation_date": props.map(|p| p.creation_date),
        "permissions": permissions,
        "sale_type": props.map(|p| p.sale_type),
        "sale_price": props.map(|p| p.sale_price),
        "position": [obj.position.x, obj.position.y, obj.position.z],
        "rotation_quat": [obj.rotation.x, obj.rotation.y, obj.rotation.z, obj.rotation.w],
        "rotation_deg": [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
        "scale": [obj.scale.x, obj.scale.y, obj.scale.z],
        "velocity": [obj.velocity.x, obj.velocity.y, obj.velocity.z],
        "hover_text": {
            "text": obj.text,
            "color": obj.text_color,
        },
        "click_action": obj.click_action,
        "prim_material": obj.prim_material,
        "flags": {
            "physical": (obj.update_flags & crate::build::edits::FLAGS_USE_PHYSICS) != 0,
            "phantom": (obj.update_flags & crate::build::edits::FLAGS_PHANTOM) != 0,
            "temporary": (obj.update_flags & crate::build::edits::FLAGS_TEMPORARY_ON_REZ) != 0,
            "is_mesh": obj.visual_volume().is_mesh(),
            "is_sculpt": obj.volume.is_sculpt(),
            "animated_mesh": super::object_is_animated_mesh(obj),
            "flexible": obj.extra.flexible.is_some(),
        },
        "physics": phys.map(|p| json!({
            "shape_type": match p.shape_type { 1 => "none", 2 => "convex_hull", _ => "prim" },
            "density": p.density,
            "friction": p.friction,
            "restitution": p.restitution,
            "gravity_multiplier": p.gravity_multiplier,
        })),
        "land_impact": land_impact,
        "sculpt": obj.extra.sculpt.map(|s| json!({
            "texture_id": ids.id(s.texture).to_string(),
            "sculpt_type": s.kind(),
            "invert": s.invert(),
            "mirror": s.mirror(),
        })),
        "light": obj.extra.light.map(|l| json!({
            "color": l.color,
            "radius": l.radius,
            "cutoff": l.cutoff,
            "falloff": l.falloff,
        })),
        "media_url": obj.media_url,
        "media_faces": obj_media.map(media_faces_to_json),
        "faces": faces,
    }))
}

fn media_faces_to_json(data: &crate::media::entry::ObjectMediaData) -> Value {
    let entries: Vec<Value> = data
        .faces
        .iter()
        .enumerate()
        .filter_map(|(face, opt)| {
            opt.as_ref().map(|e| {
                json!({
                    "face": face,
                    "current_url": e.current_url,
                    "home_url": e.home_url,
                    "auto_play": e.auto_play,
                    "auto_loop": e.auto_loop,
                    "auto_scale": e.auto_scale,
                    "auto_zoom": e.auto_zoom,
                    "first_click_interact": e.first_click_interact,
                    "width_pixels": e.width_pixels,
                    "height_pixels": e.height_pixels,
                    "controls": e.controls,
                    "whitelist_enable": e.whitelist_enable,
                    "whitelist": e.whitelist,
                })
            })
        })
        .collect();
    json!(entries)
}

fn ray_sphere_may_hit(origin: Vec3, dir: Vec3, center: Vec3, radius: f32, max_dist: f32) -> bool {
    let to = center - origin;
    let proj = to.dot(dir);
    if proj + radius < 0.0 || proj - radius > max_dist {
        return false;
    }
    let perp_sq = (to.length_squared() - proj * proj).max(0.0);
    perp_sq <= radius * radius
}

impl App {
    pub(super) fn handle_mcp_raycast(
        &mut self,
        mode: &str,
        origin_arg: Option<[f32; 3]>,
        target_arg: Option<[f32; 3]>,
        dir_arg: Option<[f32; 3]>,
        max_distance: f32,
        include_avatars: bool,
        include_terrain: bool,
        include_phantom: bool,
    ) -> Result<ViewerResponse, McpError> {
        let (origin, dir) = match mode.to_ascii_lowercase().as_str() {
            "avatar" => {
                let o = self.world.agent.position + Vec3::Z * 0.7;
                let d = (self.world.agent.body_rotation() * Vec3::X).normalize_or(Vec3::X);
                (o, d)
            }
            "points" | "ray" => {
                let o = origin_arg.map(Vec3::from_array).unwrap_or(self.camera.position);
                let d = if let Some(tgt) = target_arg {
                    (Vec3::from_array(tgt) - o).normalize_or(Vec3::X)
                } else if let Some(raw_d) = dir_arg {
                    Vec3::from_array(raw_d).normalize_or(Vec3::X)
                } else {
                    (self.camera.target - self.camera.position).normalize_or(Vec3::X)
                };
                (o, d)
            }
            _ => {
                let o = self.camera.position;
                let d = (self.camera.target - self.camera.position).normalize_or(Vec3::X);
                (o, d)
            }
        };

        struct HitCandidate {
            distance: f32,
            hit_kind: &'static str,
            pos: Vec3,
            normal: Vec3,
            local_id: Option<u32>,
            object_uuid: Option<Uuid>,
            avatar_uuid: Option<Uuid>,
            name: Option<String>,
            face: Option<i32>,
            uv: Option<[f32; 2]>,
            st: Option<[f32; 2]>,
        }

        let mut best: Option<HitCandidate> = None;
        let consider = |best: &mut Option<HitCandidate>, cand: HitCandidate| {
            if cand.distance <= max_distance && best.as_ref().is_none_or(|b| cand.distance < b.distance) {
                *best = Some(cand);
            }
        };

        // 1. Triangle-accurate GPU/CPU scene raycast when populated.
        let mut scene_candidates = Vec::new();
        self.scene.pick.ray_candidates((origin, dir), &mut scene_candidates);
        let mut tested_slots = HashSet::new();
        for idx in scene_candidates {
            let Some(obj) = self.world.objects.get(idx) else { continue };
            if obj.is_avatar() || (!include_phantom && (obj.update_flags & crate::build::edits::FLAGS_PHANTOM) != 0) {
                continue;
            }
            if let Some(surface) = self
                .scene
                .edit_touch_surface(&self.world, idx, (origin, dir))
                .or_else(|| self.scene.touch_surface(&self.world, idx, (origin, dir)))
            {
                tested_slots.insert(idx);
                let reg_off = self.world.region_offset(obj.key.region).unwrap_or(Vec3::ZERO);
                let hit_pos = surface.position + reg_off;
                let dist = (hit_pos - origin).dot(dir);
                if dist >= 0.0 {
                    consider(
                        &mut best,
                        HitCandidate {
                            distance: dist,
                            hit_kind: "object",
                            pos: hit_pos,
                            normal: surface.normal,
                            local_id: Some(obj.key.local_id),
                            object_uuid: Some(obj.full_id),
                            avatar_uuid: None,
                            name: self.build.props.get(&obj.full_id).map(|p| p.name.clone()),
                            face: Some(surface.face),
                            uv: Some([surface.uv.x, surface.uv.y]),
                            st: Some([surface.st.x, surface.st.y]),
                        },
                    );
                }
            }
        }

        let obb_hit = |obj: &crate::world::objects::Object, cap: f32| {
            let radius = obj.scale.length() * 0.5 + 1e-3;
            if !ray_sphere_may_hit(origin, dir, obj.position, radius, cap) {
                return None;
            }
            ray_box_hit(origin, dir, obj.position, obj.rotation, obj.scale * 0.5)
        };

        // 2a. Avatars (using indexed avatar slots instead of scanning all prims).
        if include_avatars {
            self.world.objects.refresh_labelled();
            for (idx, obj) in self.world.objects.avatars() {
                if tested_slots.contains(&idx) || obj.full_id == self.world.agent_id {
                    continue;
                }
                let cap = best.as_ref().map_or(max_distance, |b| b.distance);
                if let Some((dist, normal)) = obb_hit(obj, cap) {
                    consider(
                        &mut best,
                        HitCandidate {
                            distance: dist,
                            hit_kind: "avatar",
                            pos: origin + dir * dist,
                            normal,
                            local_id: Some(obj.key.local_id),
                            object_uuid: None,
                            avatar_uuid: Some(obj.full_id),
                            name: Some(self.world.social.name_of(&obj.full_id)),
                            face: None,
                            uv: None,
                            st: None,
                        },
                    );
                }
            }
        }

        // 2b. OBB fallback over unindexed world objects (headless/DemoSim + not yet in PickIndex).
        for (idx, obj) in self.world.objects.iter() {
            if obj.is_avatar() || tested_slots.contains(&idx) || self.scene.pick.is_indexed(idx) {
                continue;
            }
            if !include_phantom && (obj.update_flags & crate::build::edits::FLAGS_PHANTOM) != 0 {
                continue;
            }
            let cap = best.as_ref().map_or(max_distance, |b| b.distance);
            if let Some((dist, normal)) = obb_hit(obj, cap) {
                consider(
                    &mut best,
                    HitCandidate {
                        distance: dist,
                        hit_kind: "object",
                        pos: origin + dir * dist,
                        normal,
                        local_id: Some(obj.key.local_id),
                        object_uuid: Some(obj.full_id),
                        avatar_uuid: None,
                        name: self.build.props.get(&obj.full_id).map(|p| p.name.clone()),
                        face: None,
                        uv: None,
                        st: None,
                    },
                );
            }
        }

        // 3. Optional terrain heightmap ray-march.
        if include_terrain {
            let cap = best.as_ref().map_or(max_distance, |b| b.distance);
            let mut t = 0.0_f32;
            let step = 1.0_f32;
            while t <= cap {
                let p = origin + dir * t;
                if let Some(gh) = self.world.ground_height(p)
                    && p.z <= gh
                {
                    let mut lo = (t - step).max(0.0);
                    let mut hi = t;
                    for _ in 0..8 {
                        let mid = (lo + hi) * 0.5;
                        let mp = origin + dir * mid;
                        if self.world.ground_height(mp).is_some_and(|h| mp.z <= h) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    let hp = origin + dir * hi;
                    let hx0 = self.world.ground_height(hp - Vec3::X * 0.5).unwrap_or(hp.z);
                    let hx1 = self.world.ground_height(hp + Vec3::X * 0.5).unwrap_or(hp.z);
                    let hy0 = self.world.ground_height(hp - Vec3::Y * 0.5).unwrap_or(hp.z);
                    let hy1 = self.world.ground_height(hp + Vec3::Y * 0.5).unwrap_or(hp.z);
                    let normal = Vec3::new(hx0 - hx1, hy0 - hy1, 1.0).normalize_or(Vec3::Z);
                    consider(
                        &mut best,
                        HitCandidate {
                            distance: hi,
                            hit_kind: "terrain",
                            pos: hp,
                            normal,
                            local_id: None,
                            object_uuid: None,
                            avatar_uuid: None,
                            name: Some("Terrain".to_owned()),
                            face: None,
                            uv: None,
                            st: None,
                        },
                    );
                    break;
                }
                t += step;
            }
        }

        Ok(ViewerResponse::Json(match best {
            Some(h) => json!({
                "ok": true,
                "hit": true,
                "hit_kind": h.hit_kind,
                "distance": (h.distance * 1000.0).round() / 1000.0,
                "position": [h.pos.x, h.pos.y, h.pos.z],
                "normal": [h.normal.x, h.normal.y, h.normal.z],
                "local_id": h.local_id,
                "object_uuid": h.object_uuid.map(|u| u.to_string()),
                "avatar_uuid": h.avatar_uuid.map(|u| u.to_string()),
                "name": h.name,
                "face": h.face,
                "uv": h.uv,
                "st": h.st,
            }),
            None => json!({
                "ok": true,
                "hit": false,
                "hit_kind": Value::Null,
                "distance": Value::Null,
                "position": Value::Null,
                "normal": Value::Null,
                "local_id": Value::Null,
                "object_uuid": Value::Null,
                "face": Value::Null,
            }),
        }))
    }

    pub(super) fn start_mcp_object_inspect(
        &mut self,
        local_id: Option<u32>,
        object_uuid: Option<Uuid>,
        fetch: bool,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let idx_res = if let Some(lid) = local_id {
            self.resolve_prim_by_local_id(lid)
        } else if let Some(uid) = object_uuid {
            match self.world.objects.index_of_uuid(&uid) {
                Some(i) => {
                    let lid = self.world.objects.get(i).map(|o| o.key.local_id).unwrap_or(0);
                    self.resolve_prim_by_local_id(lid)
                }
                None => Err(McpError::InvalidParams(format!("Object {uid} not found in region"))),
            }
        } else {
            Err(McpError::InvalidParams("provide 'local_id' or 'object_uuid'".into()))
        };

        let (idx, root_uuid) = match idx_res {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        let Some(obj) = self.world.objects.get(idx) else {
            let _ = reply_tx.send(Err(McpError::InvalidParams("Object not found".into())));
            return;
        };
        let lid = obj.key.local_id;
        let full_id = obj.full_id;
        let handle = obj.key.region;
        let has_media_url = !obj.media_url.is_empty();

        let needs_props = fetch && !self.build.props.contains_key(&full_id);
        let needs_cost = fetch && !self.build.costs.objects.contains_key(&root_uuid);
        let needs_media = fetch && has_media_url && self.build.object_media(&self.media, &full_id).is_none();

        if !needs_props && !needs_cost && !needs_media {
            if let Some(payload) = serialize_object_inspect(&self.world, &self.build, &self.media, idx) {
                let _ = reply_tx.send(Ok(ViewerResponse::Json(payload)));
            } else {
                let _ = reply_tx.send(Err(McpError::InvalidParams("Object disappeared".into())));
            }
            return;
        }

        if needs_props {
            self.send(NetCommand::RequestObjectProperties { handle, object: full_id });
        }
        if needs_cost {
            let _ = self.build.hover_land_impact(handle, root_uuid);
        }
        if needs_media {
            self.build.media_get(&self.world, idx);
        }
        self.flush_build();

        self.mcp_bridge.pending_object_inspects.push_back(PendingObjectInspect {
            local_id: lid,
            object_uuid: full_id,
            root_uuid,
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(200, 10_000)),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_object_inspect(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_object_inspects) {
            let ready = self.build.props.contains_key(&p.object_uuid) && self.build.costs.objects.contains_key(&p.root_uuid);
            if ready || now.duration_since(p.started) >= p.deadline {
                let resp = self
                    .world
                    .objects
                    .index_of_uuid(&p.object_uuid)
                    .or_else(|| {
                        self.world.main_region.and_then(|region| {
                            self.world.objects.index_of(&crate::world::objects::ObjKey {
                                region,
                                local_id: p.local_id,
                            })
                        })
                    })
                    .and_then(|idx| serialize_object_inspect(&self.world, &self.build, &self.media, idx))
                    .ok_or_else(|| McpError::InvalidParams(format!("Object {} no longer present", p.object_uuid)));
                let _ = p.reply_tx.send(resp.map(ViewerResponse::Json));
            } else {
                self.mcp_bridge.pending_object_inspects.push_back(p);
            }
        }
    }

    pub(super) fn handle_mcp_region_info(&self, x: Option<f32>, y: Option<f32>) -> Result<ViewerResponse, McpError> {
        if !self.world.rlva_can_view_region() || self.world.rlva.has("showloc") {
            return Err(McpError::PermissionDenied(
                "RLVa restriction prevents inspecting region location information".into(),
            ));
        }
        let Some(main_h) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let Some(main_r) = self.world.regions.get(&main_h) else {
            return Err(McpError::NotInWorld);
        };
        let sx = x.unwrap_or(self.world.agent.position.x);
        let sy = y.unwrap_or(self.world.agent.position.y);
        let main_json = serialize_region_summary(main_r, Some([sx, sy]), &self.world);

        let neighbors: Vec<Value> = self
            .world
            .regions
            .iter()
            .filter(|(h, _)| **h != main_h)
            .map(|(_, r)| serialize_region_summary(r, None, &self.world))
            .collect();

        let restart = self.world.region_restart.alert.as_ref().map(|a| {
            json!({
                "live": a.live(),
                "seconds_remaining": a.seconds(),
                "total_seconds": a.total,
                "phase": format!("{:?}", a.phase),
            })
        });

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "region": main_json,
            "sim_fps": self.world.sim_fps,
            "restart": restart,
            "neighbors": neighbors,
        })))
    }

    pub(super) fn start_mcp_map_lookup(
        &mut self,
        name: Option<String>,
        grid_x: Option<u16>,
        grid_y: Option<u16>,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if let Some(ref q) = name {
            if let Some((gx, gy)) = self.world.map.region_by_name(q)
                && let Some((_, sim)) = self.world.map.sim_at(gx, gy)
            {
                let handle: RegionHandle = ((u64::from(gx) * 256) << 32) | (u64::from(gy) * 256);
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "matches": [{
                        "name": sim.name,
                        "grid_x": gx,
                        "grid_y": gy,
                        "handle": handle,
                        "access": sim.access,
                        "access_label": sim.access_label(),
                        "flags": sim.flags,
                        "flag_labels": sim.flag_labels(),
                        "size_x": sim.size_x,
                        "size_y": sim.size_y,
                        "is_down": sim.is_down(),
                    }],
                }))));
                return;
            }
            self.world.map.search_region(q);
        } else if let (Some(gx), Some(gy)) = (grid_x, grid_y) {
            if let Some(((ax, ay), sim)) = self.world.map.sim_at(u32::from(gx), u32::from(gy)) {
                let handle: RegionHandle = ((u64::from(ax) * 256) << 32) | (u64::from(ay) * 256);
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "matches": [{
                        "name": sim.name,
                        "grid_x": ax,
                        "grid_y": ay,
                        "handle": handle,
                        "access": sim.access,
                        "access_label": sim.access_label(),
                        "flags": sim.flags,
                        "flag_labels": sim.flag_labels(),
                        "size_x": sim.size_x,
                        "size_y": sim.size_y,
                        "is_down": sim.is_down(),
                    }],
                }))));
                return;
            }
            self.send(NetCommand::MapBlockRequest {
                min_x: gx,
                min_y: gy,
                max_x: gx,
                max_y: gy,
                null_sims: true,
            });
        }
        for cmd in self.world.map.take_commands() {
            self.send(cmd);
        }

        self.mcp_bridge.pending_map_lookups.push_back(PendingMapLookup {
            name,
            coords: grid_x.zip(grid_y),
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(300, 15_000)),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_map_lookup(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_map_lookups) {
            for cmd in self.world.map.take_commands() {
                self.send(cmd);
            }
            let found: Vec<Value> = if let Some(ref q) = p.name {
                self.world
                    .map
                    .search_results()
                    .into_iter()
                    .filter(|(_, s)| s.name.eq_ignore_ascii_case(q) || s.name.to_ascii_lowercase().contains(&q.to_ascii_lowercase()))
                    .take(25)
                    .map(|((gx, gy), s)| {
                        let handle: RegionHandle = ((u64::from(gx) * 256) << 32) | (u64::from(gy) * 256);
                        json!({
                            "name": s.name,
                            "grid_x": gx,
                            "grid_y": gy,
                            "handle": handle,
                            "access": s.access,
                            "access_label": s.access_label(),
                            "flags": s.flags,
                            "flag_labels": s.flag_labels(),
                            "size_x": s.size_x,
                            "size_y": s.size_y,
                            "is_down": s.is_down(),
                        })
                    })
                    .collect()
            } else if let Some((gx, gy)) = p.coords {
                self.world
                    .map
                    .sim_at(u32::from(gx), u32::from(gy))
                    .map(|((ax, ay), s)| {
                        let handle: RegionHandle = ((u64::from(ax) * 256) << 32) | (u64::from(ay) * 256);
                        vec![json!({
                            "name": s.name,
                            "grid_x": ax,
                            "grid_y": ay,
                            "handle": handle,
                            "access": s.access,
                            "access_label": s.access_label(),
                            "flags": s.flags,
                            "flag_labels": s.flag_labels(),
                            "size_x": s.size_x,
                            "size_y": s.size_y,
                            "is_down": s.is_down(),
                        })]
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            if !found.is_empty() || now.duration_since(p.started) >= p.deadline {
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": !found.is_empty(),
                    "timed_out": found.is_empty(),
                    "count": found.len(),
                    "matches": found,
                }))));
            } else {
                self.mcp_bridge.pending_map_lookups.push_back(p);
            }
        }
    }

    pub(super) fn start_mcp_search_people(
        &mut self,
        query: Option<String>,
        resolve_uuids: Vec<Uuid>,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if !resolve_uuids.is_empty() {
            let mut missing = Vec::new();
            for &id in &resolve_uuids {
                if self.world.social.avatar_names.complete(&id).is_none() {
                    self.world.social.want_name(id);
                    missing.push(id);
                }
            }
            if !missing.is_empty() {
                self.send(NetCommand::RequestNames(missing.clone()));
                self.send(NetCommand::RequestDisplayNames(missing));
            }
        }
        if let Some(ref q) = query {
            self.world.search.find(
                wsearch::Request::People { text: q.clone() },
                self.world.agent_access(),
                Instant::now(),
            );
            for cmd in self.world.search.take_commands() {
                self.send(cmd);
            }
        }

        let resolved_ready = resolve_uuids
            .iter()
            .all(|id| self.world.social.avatar_names.complete(id).is_some() || self.world.social.names.contains_key(id));
        let query_ready = query.as_ref().is_none_or(|_| {
            self.world
                .search
                .list(wsearch::Tab::People)
                .is_some_and(|l| l.status != wsearch::Status::Searching)
        });
        if resolved_ready && query_ready {
            let _ = reply_tx.send(Ok(ViewerResponse::Json(self.build_search_reply(
                wsearch::Tab::People,
                query.as_deref(),
                &resolve_uuids,
                false,
            ))));
            return;
        }

        self.mcp_bridge.pending_searches.push_back(PendingSearch {
            tab: wsearch::Tab::People,
            query,
            resolve_uuids,
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(300, 15_000)),
            reply_tx,
        });
    }

    pub(super) fn start_mcp_search_places(
        &mut self,
        query: String,
        category: i8,
        pg: bool,
        mature: bool,
        adult: bool,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let ratings = aurora_net::search::Ratings { pg, mature, adult };
        self.world.search.find(
            wsearch::Request::Places {
                text: query.clone(),
                ratings,
                category,
            },
            self.world.agent_access(),
            Instant::now(),
        );
        for cmd in self.world.search.take_commands() {
            self.send(cmd);
        }

        if self
            .world
            .search
            .list(wsearch::Tab::Places)
            .is_some_and(|l| l.status != wsearch::Status::Searching)
        {
            let _ = reply_tx.send(Ok(ViewerResponse::Json(self.build_search_reply(
                wsearch::Tab::Places,
                Some(&query),
                &[],
                false,
            ))));
            return;
        }

        self.mcp_bridge.pending_searches.push_back(PendingSearch {
            tab: wsearch::Tab::Places,
            query: Some(query),
            resolve_uuids: Vec::new(),
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(300, 15_000)),
            reply_tx,
        });
    }

    fn build_search_reply(&self, tab: wsearch::Tab, query: Option<&str>, resolve_uuids: &[Uuid], timed_out: bool) -> Value {
        let resolved: serde_json::Map<String, Value> = resolve_uuids
            .iter()
            .map(|id| {
                let info = self.world.social.avatar_names.get(id);
                let complete = self
                    .world
                    .social
                    .avatar_names
                    .complete(id)
                    .or_else(|| self.world.social.names.get(id).cloned());
                (
                    id.to_string(),
                    json!({
                        "complete_name": complete,
                        "display_name": info.map(|n| n.display(&self.world.social.avatar_names.options)),
                        "legacy_name": info.map(|n| n.legacy_name()).or_else(|| self.world.social.names.get(id).cloned()),
                        "username": info.map(|n| n.user_name()),
                    }),
                )
            })
            .collect();

        let (status, rows) = if query.is_some() {
            if let Some(list) = self.world.search.list(tab) {
                let items: Vec<Value> = list
                    .sorted()
                    .into_iter()
                    .map(|row| {
                        let key_val = match row.key {
                            wsearch::Key::Id(u) => json!(u.to_string()),
                            wsearch::Key::Event(e) => json!(e),
                        };
                        let vals: Vec<Value> = row.values.iter().map(search_val_to_json).collect();
                        json!({
                            "id": key_val,
                            "name": row.name,
                            "values": vals,
                        })
                    })
                    .collect();
                (search_status_str(&list.status), items)
            } else {
                ("idle", Vec::new())
            }
        } else {
            ("ok", Vec::new())
        };

        json!({
            "ok": !timed_out,
            "timed_out": timed_out,
            "status": status,
            "count": rows.len(),
            "results": rows,
            "resolved": Value::Object(resolved),
        })
    }

    pub(super) fn poll_mcp_search(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_searches) {
            let resolved_ready = p
                .resolve_uuids
                .iter()
                .all(|id| self.world.social.avatar_names.complete(id).is_some() || self.world.social.names.contains_key(id));
            let query_ready = p.query.as_ref().is_none_or(|_| {
                self.world
                    .search
                    .list(p.tab)
                    .is_some_and(|l| l.status != wsearch::Status::Searching)
            });
            let timed_out = now.duration_since(p.started) >= p.deadline;
            if (resolved_ready && query_ready) || timed_out {
                let payload = self.build_search_reply(
                    p.tab,
                    p.query.as_deref(),
                    &p.resolve_uuids,
                    timed_out && !(resolved_ready && query_ready),
                );
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(payload)));
            } else {
                self.mcp_bridge.pending_searches.push_back(p);
            }
        }
    }

    pub(super) fn start_mcp_avatar_profile(
        &mut self,
        agent_id: Uuid,
        refresh: bool,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if !refresh && let Some(p) = self.world.profiles.get(&agent_id) {
            let _ = reply_tx.send(Ok(ViewerResponse::Json(serialize_profile(p, &self.world))));
            return;
        }
        if refresh {
            self.world.profiles.refresh(agent_id);
        } else {
            self.world.profiles.want(agent_id);
        }
        for id in self.world.profiles.take_requests() {
            self.send(NetCommand::RequestProfile(id));
        }
        self.mcp_bridge.pending_profiles.push_back(PendingProfile {
            agent_id,
            started: Instant::now(),
            deadline: Duration::from_millis(timeout_ms.clamp(300, 15_000)),
            reply_tx,
        });
    }

    pub(super) fn poll_mcp_profile(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_profiles) {
            if let Some(prof) = self.world.profiles.get(&p.agent_id) {
                let _ = p.reply_tx.send(Ok(ViewerResponse::Json(serialize_profile(prof, &self.world))));
            } else if now.duration_since(p.started) >= p.deadline {
                let _ = p.reply_tx.send(Err(McpError::Timeout(p.deadline.as_millis() as u64)));
            } else {
                self.mcp_bridge.pending_profiles.push_back(p);
            }
        }
    }

    pub(super) fn start_mcp_media_info(
        &mut self,
        local_id: Option<u32>,
        object_uuid: Option<Uuid>,
        timeout_ms: u64,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let resolved = if let Some(lid) = local_id {
            match self.resolve_prim_by_local_id(lid) {
                Ok((idx, _)) => self
                    .world
                    .objects
                    .get(idx)
                    .map(|o| (idx, o.key.local_id, o.full_id, !o.media_url.is_empty())),
                Err(e) => {
                    let _ = reply_tx.send(Err(e));
                    return;
                }
            }
        } else if let Some(uid) = object_uuid {
            match self.world.objects.index_of_uuid(&uid) {
                Some(idx) => self
                    .world
                    .objects
                    .get(idx)
                    .map(|o| (idx, o.key.local_id, o.full_id, !o.media_url.is_empty())),
                None => {
                    let _ = reply_tx.send(Err(McpError::InvalidParams(format!("Object {uid} not found"))));
                    return;
                }
            }
        } else {
            None
        };

        if let Some((idx, _, full_id, has_url)) = resolved
            && has_url
            && self.build.object_media(&self.media, &full_id).is_none()
        {
            self.build.media_get(&self.world, idx);
            self.flush_build();
            self.mcp_bridge.pending_media_info.push_back(PendingMediaInfo {
                local_id: Some(self.world.objects.get(idx).unwrap().key.local_id),
                object_uuid: Some(full_id),
                started: Instant::now(),
                deadline: Duration::from_millis(timeout_ms.clamp(200, 10_000)),
                reply_tx,
            });
            return;
        }

        let _ = reply_tx.send(Ok(ViewerResponse::Json(
            self.build_media_info_json(local_id, resolved.map(|r| r.2)),
        )));
    }

    pub(super) fn poll_mcp_media_info(&mut self) {
        let now = Instant::now();
        for p in std::mem::take(&mut self.mcp_bridge.pending_media_info) {
            let ready = p.object_uuid.is_none_or(|id| self.build.object_media(&self.media, &id).is_some());
            if ready || now.duration_since(p.started) >= p.deadline {
                let _ = p
                    .reply_tx
                    .send(Ok(ViewerResponse::Json(self.build_media_info_json(p.local_id, p.object_uuid))));
            } else {
                self.mcp_bridge.pending_media_info.push_back(p);
            }
        }
    }

    fn build_media_info_json(&self, local_id: Option<u32>, object_uuid: Option<Uuid>) -> Value {
        let ids = crate::world::inventory::asset_ids::AssetIds::of(&self.world);
        let parcel_media = self.world.parcel.as_ref().map(|p| {
            json!({
                "parcel_name": p.name,
                "music_url": p.music_url,
                "media_url": p.media_url,
                "media_id": ids.id(p.media.media_id).to_string(),
            })
        });
        let prim_media = object_uuid.map(|id| {
            let obj_url = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| self.world.objects.get(i))
                .map(|o| o.media_url.clone())
                .unwrap_or_default();
            json!({
                "local_id": local_id,
                "object_uuid": id.to_string(),
                "media_url": obj_url,
                "faces": self.build.object_media(&self.media, &id).map(media_faces_to_json),
            })
        });
        json!({
            "ok": true,
            "parcel_media": parcel_media,
            "prim_media": prim_media,
        })
    }
}

fn serialize_profile(p: &aurora_net::profile::AvatarProfile, world: &World) -> Value {
    let ids = crate::world::inventory::asset_ids::AssetIds::of(world);
    let groups: Vec<Value> = p
        .groups
        .iter()
        .map(|g| {
            json!({
                "group_id": g.id.to_string(),
                "name": g.name,
                "insignia_id": ids.id(g.insignia).to_string(),
            })
        })
        .collect();
    let picks: Vec<Value> = p
        .picks
        .iter()
        .map(|(id, name)| json!({ "pick_id": id.to_string(), "name": name }))
        .collect();
    json!({
        "ok": true,
        "agent_id": p.id.to_string(),
        "name": world.social.name_of(&p.id),
        "sl_about": p.sl_about,
        "fl_about": p.fl_about,
        "sl_image_id": ids.id(p.sl_image).to_string(),
        "fl_image_id": ids.id(p.fl_image).to_string(),
        "partner_id": (!p.partner.is_nil()).then(|| p.partner.to_string()),
        "born_unix": p.born,
        "hide_age": p.hide_age,
        "customer_type": p.customer_type,
        "caption_text": p.caption_text,
        "online": p.online,
        "allow_publish": p.allow_publish(),
        "notes": p.notes,
        "groups": groups,
        "picks": picks,
    })
}
