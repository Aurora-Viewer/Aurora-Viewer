//! Changes to the selected objects, as LLSelectMgr's selection* functions
//! make them (indra/newview/llselectmgr.cpp, llviewerobject.cpp, originally
//! LGPL 2.1): each change is applied to the local copy at once (the scene
//! shows it before the simulator answers) and sent to the simulator.

use super::{BuildTool, family};
use crate::world::World;
use aurora_net::RegionHandle;
use aurora_net::build::{BuildCmd, PhysicsParams, perm, perm_field};
use aurora_prim::extra::{self, FlexibleParams, LightImageParams, LightParams, ReflectionProbeParams};
use aurora_prim::params::SculptParams;
use aurora_prim::te::TextureFace;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// update_flags bits (llprimitive.h FLAGS_*).
pub const FLAGS_USE_PHYSICS: u32 = 1 << 0;
pub const FLAGS_PHANTOM: u32 = 1 << 10;
pub const FLAGS_INCLUDE_IN_SEARCH: u32 = 1 << 15;
pub const FLAGS_TEMPORARY_ON_REZ: u32 = 1 << 29;
/// CLICK_ACTION_* (indra_constants.h).
pub const CLICK_ACTION_TOUCH: u8 = 0;
pub const CLICK_ACTION_SIT: u8 = 1;
pub const CLICK_ACTION_BUY: u8 = 2;

/// What a capability request was for (BuildTool::caps).
#[derive(Debug, Clone)]
pub enum CapPurpose {
    /// GetObjectCost for these objects.
    ObjectCost(Vec<Uuid>),
    /// ResourceCostSelected for the current selection.
    SelectionCost,
    /// GetObjectPhysicsData for these objects.
    PhysicsData(Vec<Uuid>),
    /// ObjectMedia GET of an object.
    MediaGet(Uuid),
    /// ObjectMedia UPDATE.
    MediaUpdate,
    /// ModifyMaterialParams (GLTF materials and overrides).
    GltfMaterials,
    /// RenderMaterials PUT (normal / specular maps).
    LegacyMaterials,
}

/// One extra parameter block, with its new value (`None`: not in use).
#[derive(Debug, Clone, Copy)]
pub enum Extra {
    Flexible(Option<FlexibleParams>),
    Light(Option<LightParams>),
    LightImage(Option<LightImageParams>),
    Sculpt(Option<SculptParams>),
    /// LLExtendedMeshParams flags (animated mesh).
    ExtendedMesh(Option<u32>),
    Probe(Option<ReflectionProbeParams>),
}

/// LLFlexibleObjectData defaults (llprimitive.h).
pub const FLEX_DEFAULT: FlexibleParams = FlexibleParams {
    softness: 2,
    tension: 1.0,
    air_friction: 2.0,
    gravity: 0.3,
    wind_sensitivity: 0.0,
    user_force: glam::Vec3::ZERO,
};
/// LLLightParams defaults: white, intensity 1, radius 10, falloff 0.75.
pub const LIGHT_DEFAULT: LightParams = LightParams {
    color: [1.0, 1.0, 1.0, 1.0],
    radius: 10.0,
    cutoff: 0.0,
    falloff: 0.75,
};
/// LLLightImageParams defaults: (fov π/2, focus 0, ambiance 0).
pub const LIGHT_IMAGE_DEFAULT: LightImageParams = LightImageParams {
    texture: Uuid::nil(),
    params: glam::Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
};
pub const PROBE_DEFAULT: ReflectionProbeParams = ReflectionProbeParams {
    ambiance: 0.0,
    clip_distance: 0.0,
    flags: 0,
};
/// SCULPT_DEFAULT_TEXTURE, sculpt type sphere.
pub const SCULPT_DEFAULT: SculptParams = SculptParams {
    texture: Uuid::from_u128(0xbe293869_d0d9_0a69_5989_ad27f1946fd4),
    sculpt_type: 1,
};

impl BuildTool {
    /// A tag for a capability request.
    pub fn cap(&mut self, handle: RegionHandle, cap: &str, put: bool, body: aurora_llsd::Llsd, purpose: CapPurpose) {
        let tag = self.next_tag;
        self.next_tag += 1;
        self.caps.insert(tag, purpose);
        self.send(BuildCmd::Cap {
            handle,
            cap: cap.into(),
            put,
            body,
            tag,
        });
    }

    // ---- what is selected

    /// Every selected prim (object index), roots first within each unit.
    pub fn sel_prims(&self, world: &World) -> Vec<usize> {
        self.highlighted(world).into_iter().map(|(i, _)| i).collect()
    }

    /// Number of faces (texture entries) of a prim.
    pub fn num_faces(&self, world: &World, idx: usize) -> usize {
        let Some(o) = world.objects.get(idx) else { return 1 };
        if let Some(n) = self.face_counts.get(&o.full_id) {
            return (*n).clamp(1, aurora_prim::te::MAX_TES);
        }
        if o.volume.is_mesh() {
            8
        } else if o.volume.is_sculpt() {
            1
        } else {
            aurora_prim::volume::num_faces(&o.volume).clamp(1, aurora_prim::te::MAX_TES)
        }
    }

    /// Selected faces of a selected prim (all of them outside face mode).
    pub fn sel_faces(&self, world: &World, idx: usize) -> Vec<u8> {
        let Some(o) = world.objects.get(idx) else {
            return Vec::new();
        };
        match self.faces.get(&o.key) {
            Some(f) if !f.is_empty() => f.clone(),
            _ => (0..self.num_faces(world, idx) as u8).collect(),
        }
    }

    /// (prim index, face) of every selected face.
    pub fn sel_te(&self, world: &World) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        for idx in self.sel_prims(world) {
            for f in self.sel_faces(world, idx) {
                out.push((idx, f));
            }
        }
        out
    }

    /// Do we own this object with modify rights (props when we have them,
    /// else the update's owner)?
    pub fn can_modify(&self, world: &World, idx: usize) -> bool {
        let Some(o) = world.objects.get(idx) else { return false };
        match self.props.get(&o.full_id) {
            Some(p) => p.owner_mask & perm::MODIFY != 0 && (p.owner_id == world.agent_id || p.group_id == p.owner_id),
            None => o.owner_id == world.agent_id,
        }
    }

    fn by_region(world: &World, idxs: &[usize]) -> HashMap<RegionHandle, Vec<u32>> {
        let mut m: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
        for &i in idxs {
            if let Some(o) = world.objects.get(i) {
                m.entry(o.key.region).or_default().push(o.key.local_id);
            }
        }
        m
    }

    // ---- texture entries

    /// Change the selected faces of every modifiable selected prim, then
    /// send each changed prim's whole texture entry (LLSelectMgr::
    /// selection*Set* + LLViewerObject::sendTEUpdate).
    pub fn edit_faces(&mut self, world: &mut World, mut f: impl FnMut(&mut TextureFace, usize, u8)) {
        let tes = self.sel_te(world);
        let mut changed: Vec<usize> = Vec::new();
        for (idx, face) in tes {
            if !self.can_modify(world, idx) {
                continue;
            }
            let Some(o) = world.objects.get_mut(idx) else { continue };
            // never invent the faces of an object whose entry is unknown
            let Some(mut te) = o.te.as_deref().cloned() else { continue };
            let Some(tf) = te.faces.get_mut(face as usize) else { continue };
            let before = *tf;
            f(tf, idx, face);
            if *tf != before {
                o.te = Some(Arc::new(te));
                o.material_dirty = true;
                o.render.needs_records = true;
                if !changed.contains(&idx) {
                    changed.push(idx);
                }
            }
        }
        for idx in changed {
            self.send_te(world, idx);
        }
    }

    /// ObjectImage with the prim's whole texture entry.
    pub fn send_te(&mut self, world: &World, idx: usize) {
        let n = self.num_faces(world, idx);
        let Some(o) = world.objects.get(idx) else { return };
        let Some(te) = o.te.as_deref().cloned() else { return };
        self.send(BuildCmd::SetTextures {
            handle: o.key.region,
            local_id: o.key.local_id,
            media_url: o.media_url.clone(),
            texture_entry: aurora_prim::te::pack_texture_entry(&te, n),
        });
    }

    // ---- extra parameters

    /// Set (or turn off) an extra parameter of one prim: local copy, then
    /// ObjectExtraParams (LLViewerObject::setParameterEntry / parameterChanged).
    pub fn set_extra(&mut self, world: &mut World, idx: usize, e: Extra) {
        let Some(o) = world.objects.get_mut(idx) else { return };
        let (ty, in_use, data) = match e {
            Extra::Flexible(v) => {
                let d = v.or(o.extra.flexible).unwrap_or(FLEX_DEFAULT);
                o.extra.flexible = v;
                (extra::PARAMS_FLEXIBLE, v.is_some(), extra::pack_flexible(&d))
            }
            Extra::Light(v) => {
                let d = v.or(o.extra.light).unwrap_or(LIGHT_DEFAULT);
                o.extra.light = v;
                (extra::PARAMS_LIGHT, v.is_some(), extra::pack_light(&d))
            }
            Extra::LightImage(v) => {
                let d = v.or(o.extra.light_image).unwrap_or(LIGHT_IMAGE_DEFAULT);
                o.extra.light_image = v;
                (extra::PARAMS_LIGHT_IMAGE, v.is_some(), extra::pack_light_image(&d))
            }
            Extra::Sculpt(v) => {
                let d = v.or(o.extra.sculpt).unwrap_or(SCULPT_DEFAULT);
                o.extra.sculpt = v;
                o.volume.sculpt = v;
                o.shape_dirty = true;
                (extra::PARAMS_SCULPT, v.is_some(), extra::pack_sculpt(&d))
            }
            Extra::ExtendedMesh(v) => {
                let d = v.or(o.extra.extended_mesh_flags).unwrap_or(0);
                o.extra.extended_mesh_flags = v;
                (extra::PARAMS_EXTENDED_MESH, v.is_some(), extra::pack_extended_mesh(d))
            }
            Extra::Probe(v) => {
                let d = v.or(o.extra.reflection_probe).unwrap_or(PROBE_DEFAULT);
                o.extra.reflection_probe = v;
                (extra::PARAMS_REFLECTION_PROBE, v.is_some(), extra::pack_reflection_probe(&d))
            }
        };
        o.material_dirty = true;
        o.render.needs_records = true;
        let (handle, local_id) = (o.key.region, o.key.local_id);
        self.send(BuildCmd::SetExtraParam {
            handle,
            local_id,
            param_type: ty,
            in_use,
            data,
        });
    }

    // ---- shape, flags, physics, material

    /// ObjectShape for one prim (LLViewerObject::updateVolume + sendShapeUpdate).
    pub fn set_shape(&mut self, world: &mut World, idx: usize, v: aurora_prim::VolumeParams) {
        let Some(o) = world.objects.get_mut(idx) else { return };
        let mut v = v;
        v.constrain();
        v.sculpt = o.volume.sculpt;
        if o.volume != v {
            o.volume = v;
            o.shape_dirty = true;
        }
        let (handle, local_id) = (o.key.region, o.key.local_id);
        self.send(BuildCmd::SetShape {
            handle,
            local_ids: vec![local_id],
            shape: v.to_raw(),
        });
    }

    /// Physical / temporary / phantom of the selected roots you can modify
    /// (selectionUpdatePhysics / Temporary / Phantom + updateFlags).
    pub fn set_flags(&mut self, world: &mut World, physics: Option<bool>, temporary: Option<bool>, phantom: Option<bool>) {
        let set = |f: u32, bit: u32, on: Option<bool>| match on {
            Some(true) => f | bit,
            Some(false) => f & !bit,
            None => f,
        };
        for r in self.roots(world) {
            if !self.can_modify(world, r) {
                continue;
            }
            let Some(o) = world.objects.get_mut(r) else { continue };
            let f = set(
                set(set(o.update_flags, FLAGS_USE_PHYSICS, physics), FLAGS_TEMPORARY_ON_REZ, temporary),
                FLAGS_PHANTOM,
                phantom,
            );
            o.update_flags = f;
            let (handle, local_id) = (o.key.region, o.key.local_id);
            self.send(BuildCmd::SetFlags {
                handle,
                local_id,
                physics: f & FLAGS_USE_PHYSICS != 0,
                temporary: f & FLAGS_TEMPORARY_ON_REZ != 0,
                phantom: f & FLAGS_PHANTOM != 0,
            });
        }
    }

    /// Physics shape type and material values of every selected prim you
    /// can modify (selectionSetPhysicsType etc.: ObjectFlagUpdate with
    /// ExtraPhysics, the flags unchanged).
    pub fn set_physics(&mut self, world: &World, change: impl Fn(&mut PhysicsParams)) {
        for idx in self.sel_prims(world) {
            if !self.can_modify(world, idx) {
                continue;
            }
            let Some(o) = world.objects.get(idx) else { continue };
            let p = self.physics.entry(o.key.local_id).or_default();
            change(p);
            let params = *p;
            let f = o.update_flags;
            self.send(BuildCmd::SetPhysicsParams {
                handle: o.key.region,
                local_id: o.key.local_id,
                physics: f & FLAGS_USE_PHYSICS != 0,
                temporary: f & FLAGS_TEMPORARY_ON_REZ != 0,
                phantom: f & FLAGS_PHANTOM != 0,
                params,
            });
        }
    }

    /// ObjectMaterial (LL_MCODE_*), keeping the upper bits.
    pub fn set_material(&mut self, world: &mut World, code: u8) {
        for idx in self.sel_prims(world) {
            if !self.can_modify(world, idx) {
                continue;
            }
            let Some(o) = world.objects.get_mut(idx) else { continue };
            let m = code | (o.prim_material & !0x0f);
            o.prim_material = m;
            o.material_dirty = true;
            let (handle, local_id) = (o.key.region, o.key.local_id);
            self.send(BuildCmd::SetMaterial {
                handle,
                local_ids: vec![local_id],
                material: m,
            });
        }
    }

    // ---- properties (General tab)

    /// Roots of the selection, or the prims when only children are selected
    /// (LLPanelPermissions: SEND_ONLY_ROOTS falls back to individuals).
    fn prop_targets(&self, world: &World) -> Vec<usize> {
        let roots: Vec<usize> = self
            .sel_prims(world)
            .into_iter()
            .filter(|&i| world.objects.get(i).is_some_and(|o| o.parent_id == 0))
            .collect();
        if roots.is_empty() { self.sel_prims(world) } else { roots }
    }

    /// ObjectPermissions on the roots; the masks are updated locally.
    pub fn set_perm(&mut self, world: &World, field: u8, set: bool, mask: u32) {
        let targets = self.prop_targets(world);
        for idx in &targets {
            let Some(o) = world.objects.get(*idx) else { continue };
            if let Some(p) = self.props.get_mut(&o.full_id) {
                let m = match field {
                    perm_field::OWNER => &mut p.owner_mask,
                    perm_field::GROUP => &mut p.group_mask,
                    perm_field::EVERYONE => &mut p.everyone_mask,
                    perm_field::NEXT_OWNER => &mut p.next_owner_mask,
                    _ => &mut p.base_mask,
                };
                if set {
                    *m |= mask;
                } else {
                    *m &= !mask;
                }
            }
        }
        for (handle, local_ids) in Self::by_region(world, &targets) {
            self.send(BuildCmd::SetPermissions {
                handle,
                local_ids,
                field,
                set,
                mask,
            });
        }
    }

    /// ObjectGroup on the roots.
    pub fn set_group(&mut self, world: &World, group_id: Uuid) {
        let targets = self.prop_targets(world);
        for idx in &targets {
            if let Some(p) = world.objects.get(*idx).and_then(|o| self.props.get_mut(&o.full_id)) {
                p.group_id = group_id;
            }
        }
        for (handle, local_ids) in Self::by_region(world, &targets) {
            self.send(BuildCmd::SetGroup {
                handle,
                local_ids,
                group_id,
            });
        }
    }

    /// Deed to the group (LLSelectMgr::sendOwner(null, group, false)).
    pub fn deed(&mut self, world: &World, group_id: Uuid) {
        let targets = self.prop_targets(world);
        for (handle, local_ids) in Self::by_region(world, &targets) {
            self.send(BuildCmd::SetOwner {
                handle,
                local_ids,
                owner_id: Uuid::nil(),
                group_id,
            });
        }
    }

    /// ObjectSaleInfo on the roots (sale type 0 = not for sale).
    pub fn set_sale(&mut self, world: &World, sale_type: u8, price: i32) {
        let price = price.max(0);
        let targets = self.prop_targets(world);
        for idx in &targets {
            if let Some(p) = world.objects.get(*idx).and_then(|o| self.props.get_mut(&o.full_id)) {
                p.sale_type = sale_type;
                p.sale_price = price;
            }
        }
        for (handle, local_ids) in Self::by_region(world, &targets) {
            self.send(BuildCmd::SetSaleInfo {
                handle,
                local_ids,
                sale_type,
                price,
            });
        }
    }

    /// ObjectClickAction on every selected prim (SEND_INDIVIDUALS).
    pub fn set_click_action(&mut self, world: &mut World, action: u8) {
        let prims = self.sel_prims(world);
        for &i in &prims {
            if let Some(o) = world.objects.get_mut(i) {
                o.click_action = action;
            }
        }
        for (handle, local_ids) in Self::by_region(world, &prims) {
            self.send(BuildCmd::SetClickAction { handle, local_ids, action });
        }
    }

    /// FLAGS_INCLUDE_IN_SEARCH on the roots (setIncludeInSearch:
    /// ObjectFlagUpdate, then ObjectIncludeInSearch).
    pub fn set_include_in_search(&mut self, world: &mut World, include: bool) {
        let roots = self.roots(world);
        for &r in &roots {
            let Some(o) = world.objects.get_mut(r) else { continue };
            if include {
                o.update_flags |= FLAGS_INCLUDE_IN_SEARCH;
            } else {
                o.update_flags &= !FLAGS_INCLUDE_IN_SEARCH;
            }
            let f = o.update_flags;
            let (handle, local_id) = (o.key.region, o.key.local_id);
            self.send(BuildCmd::SetFlags {
                handle,
                local_id,
                physics: f & FLAGS_USE_PHYSICS != 0,
                temporary: f & FLAGS_TEMPORARY_ON_REZ != 0,
                phantom: f & FLAGS_PHANTOM != 0,
            });
        }
        for (handle, local_ids) in Self::by_region(world, &roots) {
            self.send(BuildCmd::SetIncludeInSearch {
                handle,
                local_ids,
                include,
            });
        }
    }

    /// Name / description of the property targets (ObjectName / ObjectDescription).
    pub fn set_name(&mut self, world: &World, name: &str, description: bool) {
        let targets = self.prop_targets(world);
        for idx in targets {
            let Some(o) = world.objects.get(idx) else { continue };
            if let Some(p) = self.props.get_mut(&o.full_id) {
                if description {
                    p.description = name.to_owned();
                } else {
                    p.name = name.to_owned();
                }
            }
            let (handle, local_id) = (o.key.region, o.key.local_id);
            self.send(if description {
                BuildCmd::SetDescription {
                    handle,
                    local_id,
                    description: name.to_owned(),
                }
            } else {
                BuildCmd::SetName {
                    handle,
                    local_id,
                    name: name.to_owned(),
                }
            });
        }
    }

    /// The selection's linksets, whole (for « Copier l'UUID », link
    /// numbers...): every prim of each selected root.
    pub fn linkset_prims(&self, world: &World) -> Vec<usize> {
        let mut out = Vec::new();
        for r in self.roots(world) {
            out.extend(family(world, r));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_pack() {
        assert_eq!(extra::pack_flexible(&FLEX_DEFAULT).len(), 16);
        assert_eq!(extra::pack_light(&LIGHT_DEFAULT).len(), 16);
        assert_eq!(extra::pack_light_image(&LIGHT_IMAGE_DEFAULT).len(), 28);
        assert_eq!(extra::pack_reflection_probe(&PROBE_DEFAULT).len(), 9);
        assert_eq!(extra::pack_sculpt(&SCULPT_DEFAULT).len(), 17);
    }
}
