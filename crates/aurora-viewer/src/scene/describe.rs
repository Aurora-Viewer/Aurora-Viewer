//! The "Debug to log" diagnostic (Preferences › Debug › Logs, Advanced menu):
//! what the scene draws of the objects around and of what the avatars near
//! us wear, face by face, so that a rendering report from the grid can be
//! read from the log alone. It only reads the scene: nothing here changes
//! what is drawn.

use super::jobs::AlphaKind;
use super::{FaceDraw, GeomState, GpuGeom, ObjGpu, Pass, Scene, avatar, classify, legacy_mat, te_invisible};
use crate::world::World;
use crate::world::objects::Object;
use aurora_prim::te::{TRANSPARENT_TEXTURE, TextureFace};
use aurora_render::Renderer;
use glam::Vec3;
use std::time::Instant;
use uuid::Uuid;

/// Avatars nearer than this (m) get every face of what they wear written,
/// when nothing is selected.
pub const DETAIL_RADIUS: f32 = 3.0;
/// Avatars whose attachments get their summary line (m).
pub const WORN_RADIUS: f32 = 10.0;
/// Prim lines per attachment and per press: a mesh body of a few hundred
/// prims keeps the log readable.
const PRIMS_PER_ATTACHMENT: usize = 64;
const PRIMS_PER_PRESS: usize = 600;

/// The name of a worn or selected object (ObjectProperties), when known.
pub type Names<'a> = &'a dyn Fn(&Uuid) -> Option<String>;

/// What a press writes face by face: the selected linksets when the build
/// tools have a selection, else what the avatars within [`DETAIL_RADIUS`]
/// wear. Taken at the press, written once the names asked have come.
#[derive(Debug, Clone, Default)]
pub struct DescribeScope {
    /// Roots of the selected linksets (object slots).
    pub selected: Vec<usize>,
    /// The avatars whose attachments are detailed.
    pub avatars: Vec<Uuid>,
}

impl DescribeScope {
    /// Roots of the linksets the press details.
    pub fn roots(&self, world: &World) -> Vec<usize> {
        let mut out = self.selected.clone();
        for id in &self.avatars {
            if let Some(av) = world.objects.index_of_uuid(id).and_then(|i| world.objects.get(i)) {
                out.extend_from_slice(world.objects.children_of(&av.key));
            }
        }
        out
    }
}

/// The prims of a linkset, root first (seated avatars left out).
fn linkset(world: &World, root: usize) -> Vec<usize> {
    let mut prims = vec![root];
    if let Some(o) = world.objects.get(root) {
        prims.extend(
            world
                .objects
                .children_of(&o.key)
                .iter()
                .copied()
                .filter(|&i| world.objects.get(i).is_some_and(|c| !c.is_avatar())),
        );
    }
    prims
}

/// The root of the linkset of a prim: the prim whose parent is an avatar
/// (an attachment) or nothing.
fn linkset_root(world: &World, mut idx: usize) -> usize {
    for _ in 0..16 {
        let Some(parent) = world.objects.get(idx).and_then(|o| world.objects.parent_of(o)) else {
            break;
        };
        if world.objects.get(parent).is_none_or(|p| p.is_avatar()) {
            break;
        }
        idx = parent;
    }
    idx
}

fn legacy_mode(mode: u8) -> &'static str {
    match mode {
        legacy_mat::alpha_mode::NONE => "none",
        legacy_mat::alpha_mode::BLEND => "blend",
        legacy_mat::alpha_mode::MASK => "mask",
        legacy_mat::alpha_mode::EMISSIVE => "emissive",
        _ => "unknown",
    }
}

fn gltf_mode(mode: u8) -> &'static str {
    match mode {
        0 => "opaque",
        1 => "blend",
        2 => "mask",
        _ => "unknown",
    }
}

/// A texture-entry color: rgb, and the alpha as received with three
/// decimals (0.004 is the smallest non-zero one).
fn rgba(c: [f32; 4]) -> String {
    format!("{:.2},{:.2},{:.2},{:.3}", c[0], c[1], c[2], c[3])
}

impl Scene {
    /// Scene-space position of an object as drawn.
    fn drawn_position(world: &World, idx: usize) -> Option<Vec3> {
        Self::object_transform(world, idx, Instant::now(), 0).map(|t| t.0)
    }

    /// The other avatars within `radius` m of ours, nearest first (drawn
    /// positions: a seated avatar's own position is relative to its seat).
    fn avatars_near(world: &World, radius: f32) -> Vec<(f32, Uuid)> {
        let Some(me) = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|i| Self::drawn_position(world, i))
        else {
            return Vec::new();
        };
        let mut near: Vec<(f32, Uuid)> = world
            .objects
            .iter()
            .filter(|(_, o)| o.is_avatar() && o.full_id != world.agent_id)
            .filter_map(|(i, o)| Some((Self::drawn_position(world, i)?.distance(me), o.full_id)))
            .filter(|(d, _)| *d < radius)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near
    }

    /// What a press of the button details (`selected`: the slots the build
    /// tools have selected).
    pub fn describe_scope(&self, world: &World, selected: &[usize]) -> DescribeScope {
        if !selected.is_empty() {
            let mut roots: Vec<usize> = Vec::new();
            for &i in selected {
                let root = linkset_root(world, i);
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
            return DescribeScope {
                selected: roots,
                avatars: Vec::new(),
            };
        }
        DescribeScope {
            selected: Vec::new(),
            avatars: Self::avatars_near(world, DETAIL_RADIUS).into_iter().map(|(_, id)| id).collect(),
        }
    }

    /// The worn part of a press: the selected linksets face by face, then
    /// the attachments of the avatars within [`WORN_RADIUS`], face by face
    /// for those of `scope`.
    pub fn describe_worn(&self, world: &World, renderer: &Renderer, scope: &DescribeScope, names: Names) {
        log::info!(
            "describe: {} selected linksets, {} avatars within {DETAIL_RADIUS} m detailed face by face",
            scope.selected.len(),
            scope.avatars.len()
        );
        let mut budget = PRIMS_PER_PRESS;
        for &root in &scope.selected {
            self.log_selected(world, renderer, root, names, &mut budget);
        }
        for (_, id) in Self::avatars_near(world, WORN_RADIUS) {
            let detail = scope.avatars.contains(&id);
            self.log_attachments(world, id, renderer, names, detail.then_some(&mut budget));
        }
    }

    /// Diagnostic: [`Self::log_attachments`] of the other avatars within
    /// `radius` m of ours, the nearest first, without the faces.
    pub fn log_attachments_near(&self, world: &World, radius: f32, renderer: &Renderer) {
        for (_, id) in Self::avatars_near(world, radius) {
            self.log_attachments(world, id, renderer, &|_| None, None);
        }
    }

    /// A selected linkset, face by face: as an attachment of its wearer, or
    /// as an object of the world.
    fn log_selected(&self, world: &World, renderer: &Renderer, root: usize, names: Names, budget: &mut usize) {
        let Some(o) = world.objects.get(root) else {
            return;
        };
        let wearer = world
            .objects
            .parent_of(o)
            .and_then(|p| world.objects.get(p))
            .filter(|p| p.is_avatar());
        match wearer {
            Some(av) => {
                log::info!("selected: worn by {}", Self::avatar_label(world, av));
                self.log_attachment(world, renderer, root, names, Some(budget));
            }
            None => {
                let prims = linkset(world, root);
                log::info!(
                    "selected object '{}' {}: {} prims",
                    names(&o.full_id).unwrap_or_else(|| "?".into()),
                    o.full_id,
                    prims.len()
                );
                let lines: Vec<String> = prims
                    .iter()
                    .filter_map(|&i| Some(self.prim_faces_line(world, renderer, world.objects.get(i)?, self.gpu.get(i)?)))
                    .collect();
                Self::log_prim_lines(&lines, Some(budget));
            }
        }
    }

    /// An avatar for the log: display and user names, legacy name, id.
    fn avatar_label(world: &World, av: &Object) -> String {
        format!(
            "'{}' (legacy name {}, {})",
            world.social.name_of(&av.full_id),
            av.display_name().unwrap_or_default(),
            av.full_id
        )
    }

    /// Diagnostic: one log line per attachment of an avatar (name, mesh /
    /// rigged, faces drawn and left out, position relative to the avatar),
    /// then the faces whose base texture does not show (not loaded, slot
    /// not the streamer's, or the renderer still holding the placeholder).
    /// `detail`: every prim's faces too, within that many prim lines left.
    pub fn log_attachments(&self, world: &World, avatar: Uuid, renderer: &Renderer, names: Names, mut detail: Option<&mut usize>) {
        let Some(aidx) = world.objects.index_of_uuid(&avatar) else {
            return;
        };
        let Some(av) = world.objects.get(aidx) else {
            return;
        };
        let me = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|i| Self::drawn_position(world, i));
        let distance = Self::drawn_position(world, aidx).zip(me).map_or(0.0, |(a, b)| a.distance(b));
        log::info!(
            "attachments of {} ({distance:.1} m away, faces {}): root offset {:.3} m, appearance params {}",
            Self::avatar_label(world, av),
            if detail.is_some() { "detailed" } else { "summed up" },
            world.avatar_shape_dz.get(&avatar).copied().unwrap_or(0.0) + world.appearance_hover(&avatar),
            world.visual_params(&avatar).map(|v| v.len()).unwrap_or(0)
        );
        for (te, name, _) in avatar::BAKES {
            if let Some(t) = world.bake_texture(aidx, te) {
                log::info!("  bake {name}: {t} — {}", self.textures.describe(&t));
            }
        }
        for &att in world.objects.children_of(&av.key) {
            self.log_attachment(world, renderer, att, names, detail.as_deref_mut());
        }
    }

    /// One attachment: its summary line, then its prims face by face when
    /// `detail` is given or when it shows the head bake.
    fn log_attachment(&self, world: &World, renderer: &Renderer, att: usize, names: Names, detail: Option<&mut usize>) {
        let Some(root) = world.objects.get(att).filter(|o| !o.is_avatar()) else {
            return;
        };
        let table = &renderer.textures;
        let wearer = world.objects.parent_of(root).and_then(|p| world.objects.get(p));
        let apos = world
            .objects
            .parent_of(root)
            .and_then(|p| Self::drawn_position(world, p))
            .unwrap_or(Vec3::ZERO);
        // our own attachments: the inventory item; the others': the name
        // ObjectProperties gave (asked by the press, app/describe.rs)
        let name = root
            .attachment_item_id()
            .and_then(|id| world.inventory.items.get(&id))
            .map(|i| i.name.clone())
            .or_else(|| names(&root.full_id))
            .unwrap_or_else(|| "?".into());
        let prims = linkset(world, att);
        // a linkset showing the head bake is always detailed
        let head_set = prims.iter().any(|&i| {
            world
                .objects
                .get(i)
                .and_then(|o| o.te.as_ref())
                .is_some_and(|t| t.faces.iter().any(|f| f.texture == avatar::BAKES[0].2))
        });
        let alpha = &self.textures.alpha_by_slot;
        let (mut meshes, mut rigged, mut faces, mut shown, mut pending, mut empty) = (0, 0, 0, 0, 0, 0);
        let (mut te_zero, mut no_tris) = (0, 0);
        let mut suspects: Vec<String> = Vec::new();
        let mut gltf_faces: Vec<String> = Vec::new();
        let mut prim_lines: Vec<String> = Vec::new();
        let mut far = 0.0f32;
        for &i in &prims {
            let Some(o) = world.objects.get(i) else {
                continue;
            };
            if o.visual_volume().is_mesh() {
                meshes += 1;
                if o.visual_volume()
                    .sculpt
                    .and_then(|s| self.meshes.meta(&s.texture))
                    .is_some_and(|m| m.skin.is_some())
                {
                    rigged += 1;
                }
            }
            let Some(g) = self.gpu.get(i) else {
                continue;
            };
            if g.geom.is_none() {
                pending += 1;
            }
            if g.faces.is_empty() {
                empty += 1;
            }
            let (zero, none) = self.left_out(o, g);
            te_zero += zero;
            no_tris += none;
            for f in &g.faces {
                let fi = f.te_index as usize;
                faces += 1;
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                if classify(f, a) != Pass::Hidden {
                    shown += 1;
                }
                let gltf = self.gltf_face_desc(world, o, fi);
                if let Some(why) = self.textures.face_problem(&f.tex_id, f.base_slot, table) {
                    suspects.push(format!(
                        "  face {} #{fi}: {why} — tex {} ({}), slot {} = {}, maps {:?}, {}",
                        o.full_id,
                        f.tex_id,
                        self.textures.describe(&f.tex_id),
                        f.base_slot,
                        table.describe(f.base_slot),
                        f.aux_tex,
                        gltf.as_deref().unwrap_or("no glTF material")
                    ));
                }
                if let Some(desc) = gltf {
                    let tf = o.te.as_ref().map(|t| *t.face(fi)).unwrap_or_default();
                    gltf_faces.push(format!(
                        "  glTF face {} #{fi}: {desc}, drawn with base {} ({}), maps {:?}, texture entry {} unused",
                        o.full_id,
                        f.tex_id,
                        self.textures.describe(&f.tex_id),
                        f.aux_tex,
                        tf.texture
                    ));
                }
            }
            if !g.faces.is_empty() {
                far = far.max(g.center.distance(apos));
            }
            if head_set || detail.is_some() {
                prim_lines.push(self.prim_faces_line(world, renderer, o, g));
            }
        }
        log::info!(
            "attachment '{name}' {} point {}: {} prims ({meshes} mesh, {rigged} rigged, {pending} without geometry, {empty} with no face drawn), faces {shown}/{faces} drawn, left out {te_zero} with texture-entry alpha 0 and {no_tris} without triangles, farthest center {far:.2} m from the avatar, root offset {:.3?} on its point",
            root.full_id,
            root.attachment_point(),
            prims.len(),
            root.position
        );
        // the head linkset is detailed outside the budget, as before
        Self::log_prim_lines(&prim_lines, detail.filter(|_| !head_set));
        // legacy materials of our own attachments (normal / specular maps)
        if wearer.is_some_and(|w| w.full_id == world.agent_id) {
            let mut seen: Vec<Uuid> = Vec::new();
            for &i in &prims {
                let Some(te) = world.objects.get(i).and_then(|o| o.te.as_ref()) else {
                    continue;
                };
                for f in &te.faces {
                    if f.material_id.is_nil() || seen.contains(&f.material_id) {
                        continue;
                    }
                    seen.push(f.material_id);
                    if let Some(lm) = self.legacy_mats.peek(&f.material_id) {
                        log::info!(
                            "  material {}: normal {} ({}) x{:.2?}, specular {} ({}) x{:.2?}, color {:?} exp {} env {} alpha mode {}",
                            f.material_id,
                            lm.normal_map,
                            self.textures.describe(&lm.normal_map),
                            &lm.normal_st[..2],
                            lm.specular_map,
                            self.textures.describe(&lm.specular_map),
                            &lm.specular_st[..2],
                            lm.specular_color,
                            lm.specular_exp,
                            lm.env_intensity,
                            lm.diffuse_alpha_mode
                        );
                    }
                }
            }
        }
        let more = suspects.len().saturating_sub(24);
        for line in suspects.iter().take(24) {
            log::info!("{line}");
        }
        if more > 0 {
            log::info!("  … {more} more faces with a texture problem");
        }
        // glTF faces: material, whether it loaded, override (they are
        // drawn with the material's base color texture, never with the
        // texture entry's)
        let more = gltf_faces.len().saturating_sub(24);
        for line in gltf_faces.iter().take(24) {
            log::info!("{line}");
        }
        if more > 0 {
            log::info!("  … {more} more glTF faces");
        }
    }

    /// Prim lines within the budget (all of them without one).
    fn log_prim_lines(lines: &[String], budget: Option<&mut usize>) {
        let shown = match budget {
            Some(left) => {
                let n = lines.len().min(PRIMS_PER_ATTACHMENT).min(*left);
                *left -= n;
                n
            }
            None => lines.len(),
        };
        for line in &lines[..shown] {
            log::info!("  {line}");
        }
        if lines.len() > shown {
            log::info!(
                "  … {} more prims not detailed (at most {PRIMS_PER_ATTACHMENT} per attachment and {PRIMS_PER_PRESS} per press)",
                lines.len() - shown
            );
        }
    }

    /// The geometry of a prim, when built.
    fn ready_geom(&self, g: &ObjGpu) -> Option<&GpuGeom> {
        match &self.geoms.get(&g.geom?)?.state {
            GeomState::Ready(geom) => Some(&**geom),
            _ => None,
        }
    }

    fn geom_state(&self, g: &ObjGpu) -> &'static str {
        match g.geom.map(|k| self.geoms.get(&k).map(|e| &e.state)) {
            None => "none",
            Some(None) => "released",
            Some(Some(GeomState::Pending)) => "pending",
            Some(Some(GeomState::Failed)) => "failed",
            Some(Some(GeomState::Ready(_))) => "ready",
        }
    }

    /// Faces of a prim that `rebuild_faces` left out: (texture-entry alpha
    /// 0 without a glTF material, LL's PASS_ALPHA_INVISIBLE; without
    /// triangles at the level of detail drawn).
    fn left_out(&self, o: &Object, g: &ObjGpu) -> (usize, usize) {
        let Some(geom) = self.ready_geom(g) else {
            return (0, 0);
        };
        let (mut zero, mut none) = (0, 0);
        for (fi, face) in geom.faces.iter().enumerate() {
            if g.faces.iter().any(|f| f.te_index as usize == fi) {
                continue;
            }
            match face {
                None => none += 1,
                Some(_) if te_invisible(o, fi) => zero += 1,
                Some(_) => {}
            }
        }
        (zero, none)
    }

    /// A texture for the log: a bake slot by name, the transparent texture
    /// called so, else its id and state.
    fn texture_label(&self, id: &Uuid) -> String {
        if let Some((_, name)) = avatar::bake_slot_for(id) {
            return format!("bake:{name}");
        }
        let transparent = if *id == TRANSPARENT_TEXTURE { "transparent texture, " } else { "" };
        format!("{id} ({transparent}{})", self.textures.describe(id))
    }

    /// One prim, every face of its texture entry: drawn (with everything
    /// that chose its pass) or left out (and why), so that a see-through or
    /// missing part can be told from a creator's choice.
    fn prim_faces_line(&self, world: &World, renderer: &Renderer, o: &Object, g: &ObjGpu) -> String {
        let geom = self.ready_geom(g);
        let te_faces = o.te.as_ref().map_or(0, |t| t.faces.len());
        let count = geom.map_or(te_faces, |geom| geom.faces.len());
        let volume = o.visual_volume();
        let kind = match (volume.is_mesh(), g.rigged, volume.sculpt.is_some()) {
            (true, true, _) => "rigged mesh",
            (true, false, _) => "mesh",
            (false, _, true) => "sculpt",
            _ => "prim",
        };
        let alpha = &self.textures.alpha_by_slot;
        let (mut zero, mut none, mut hidden) = (0, 0, 0);
        let mut line = String::new();
        for fi in 0..count {
            let tf = o.visual_texture_face(fi);
            if let Some(f) = g.faces.iter().find(|f| f.te_index as usize == fi) {
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                let pass = classify(f, a);
                if pass == Pass::Hidden {
                    hidden += 1;
                }
                line.push_str(&self.drawn_face(world, renderer, o, fi, &tf, f, a, pass));
                continue;
            }
            let why = match geom.map(|geom| geom.faces[fi].is_some()) {
                None => "no geometry",
                Some(false) => {
                    none += 1;
                    "no triangles at this level of detail"
                }
                Some(true) if te_invisible(o, fi) => {
                    zero += 1;
                    "texture-entry alpha 0"
                }
                Some(true) => "not built",
            };
            let legacy = if tf.material_id.is_nil() {
                "-".to_string()
            } else {
                tf.material_id.to_string()
            };
            let gltf = o.visual_material(fi).map_or_else(|| "-".to_string(), |id| id.to_string());
            line.push_str(&format!(
                " [{fi} left out: {why}, tex {}, rgba {}, legacy material {legacy}, glTF material {gltf}]",
                self.texture_label(&tf.texture),
                rgba(tf.color)
            ));
        }
        let drawn = if g.faces.is_empty() {
            "no faces drawn".to_string()
        } else {
            format!("{}/{count} faces drawn", g.faces.len())
        };
        format!(
            "prim {} ({kind}, lod {}, geometry {}): {drawn}, {zero} left out for texture-entry alpha 0, {none} without triangles, {hidden} hidden:{line}",
            o.full_id,
            g.lod,
            self.geom_state(g)
        )
    }

    /// A drawn face: texture entry as received, what the draw record holds,
    /// the material's alpha mode, the texture's alpha class, the pass.
    #[allow(clippy::too_many_arguments)]
    fn drawn_face(
        &self,
        world: &World,
        renderer: &Renderer,
        o: &Object,
        fi: usize,
        tf: &TextureFace,
        f: &FaceDraw,
        tex_alpha: AlphaKind,
        pass: Pass,
    ) -> String {
        let mut tex = self.texture_label(&tf.texture);
        if f.tex_id != tf.texture && !f.tex_id.is_nil() {
            tex.push_str(&format!(" drawn with {} ({})", f.tex_id, self.textures.describe(&f.tex_id)));
        }
        let record = renderer.records.get(f.record);
        // the alpha the shader multiplies by: the glTF base color factor's,
        // or the texture entry's
        let drawn_alpha = record
            .map(|r| r.base_color[3])
            .filter(|a| (a - tf.color[3]).abs() > 0.001)
            .map(|a| format!(" drawn alpha {a:.3}"))
            .unwrap_or_default();
        let cutoff = record.map_or(0.0, |r| r.params[3]);
        let material = match f.pbr_alpha {
            Some(mode) => format!(
                "glTF {} cutoff {cutoff:.2} ({})",
                gltf_mode(mode),
                self.gltf_face_desc(world, o, fi).unwrap_or_default()
            ),
            None if tf.material_id.is_nil() => "no material".to_string(),
            None => match self.legacy_mats.peek(&tf.material_id) {
                Some(lm) => format!(
                    "legacy {} alpha {} cutoff {}{}",
                    tf.material_id,
                    legacy_mode(lm.diffuse_alpha_mode),
                    lm.alpha_cutoff,
                    if f.legacy_alpha.is_none() { " (not loaded when built)" } else { "" }
                ),
                None => format!("legacy {} not loaded", tf.material_id),
            },
        };
        let cached = if f.pass != pass {
            format!(" (cached {:?})", f.pass)
        } else {
            String::new()
        };
        format!(
            " [{fi} tex {tex} rgba {}{drawn_alpha} {material} tex alpha {tex_alpha:?}{} -> {pass:?}{cached}]",
            rgba(tf.color),
            if f.two_sided { " two-sided" } else { "" }
        )
    }

    /// Diagnostic: the glTF material of a face (id, loading state, the
    /// override's base color), None for a Blinn-Phong face.
    fn gltf_face_desc(&self, world: &World, o: &Object, fi: usize) -> Option<String> {
        let mid = o
            .extra
            .render_materials
            .iter()
            .find(|(t, _)| *t as usize == fi)
            .map(|(_, id)| *id)
            .filter(|id| !id.is_nil())?;
        let ov = world
            .gltf_overrides
            .get(&o.key)
            .and_then(|s| s.iter().rev().find(|(f, _)| *f as usize == fi))
            .map(|(_, ov)| ov);
        let ov = match ov {
            Some(ov) => {
                let tex = ov.textures[aurora_assets::material::TEXTURE_BASE_COLOR];
                let tex = if tex.is_nil() {
                    "-".to_string()
                } else {
                    format!("{tex} ({})", self.textures.describe(&tex))
                };
                format!("override base tex {tex} factor {:?}", ov.base_color_factor)
            }
            None => "no override".into(),
        };
        Some(format!("glTF material {mid} ({}), {ov}", self.materials.describe(&mid)))
    }

    /// Diagnostic: the objects (not avatars nor their attachments) within
    /// `radius` m of `center`: lights and every face's texture entry, as
    /// the build floater would show them.
    pub fn log_nearby(&self, world: &World, center: Vec3, radius: f32) {
        let alpha = &self.textures.alpha_by_slot;
        let mut n = 0;
        for (idx, o) in world.objects.iter() {
            let Some(g) = self.gpu.get(idx) else {
                continue;
            };
            if g.is_avatar || g.owner_avatar.is_some() || g.hud || g.faces.is_empty() {
                continue;
            }
            if g.center.distance(center) - g.radius > radius {
                continue;
            }
            n += 1;
            log::info!(
                "object {} local {}: {} material {} scale {:.2?} center {:.1?}, {} faces, light {:?}",
                o.full_id,
                o.key.local_id,
                if o.visual_volume().is_mesh() { "mesh" } else { "prim" },
                o.prim_material,
                o.visual_scale(),
                g.center,
                g.faces.len(),
                o.extra.light
            );
            let te = o.te.as_ref();
            let mut mats: Vec<Uuid> = Vec::new();
            let mut pbr_mats: Vec<Uuid> = Vec::new();
            // a face left out (texture-entry alpha 0) shifts the drawn ones:
            // the texture entry is read at the face's own index
            for f in &g.faces {
                let fi = f.te_index as usize;
                let tf = te.map(|t| *t.face(fi)).unwrap_or_default();
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                let pbr = o.extra.render_materials.iter().find(|(t, _)| *t as usize == fi).map(|(_, id)| *id);
                log::info!(
                    "  face {fi}: tex {} ({}), color {:.2?}, glow {:.2}, fullbright {}, shiny {}, bump {}, planar {}, repeats {:.2}x{:.2}, material {}, pbr {:?}, pass {:?}",
                    tf.texture,
                    self.textures.describe(&tf.texture),
                    tf.color,
                    tf.glow,
                    tf.fullbright(),
                    tf.shiny(),
                    tf.bump(),
                    tf.planar(),
                    tf.scale_s,
                    tf.scale_t,
                    tf.material_id,
                    pbr,
                    classify(f, a)
                );
                if !tf.material_id.is_nil() && !mats.contains(&tf.material_id) {
                    mats.push(tf.material_id);
                }
                if let Some(id) = pbr.filter(|id| !id.is_nil() && !pbr_mats.contains(id)) {
                    pbr_mats.push(id);
                }
            }
            // a map of a PBR material with its load state, as a face's texture
            let map = |t: Option<Uuid>| t.map_or_else(|| "none".to_string(), |t| format!("{t} ({})", self.textures.describe(&t)));
            for id in pbr_mats {
                match self.materials.peek(&id) {
                    Some(m) => log::info!(
                        "  pbr {id}: base {}, normal {}, orm {}, emissive {}, transforms (scale, offset, rotation) {:?}",
                        map(m.base_color_texture),
                        map(m.normal_texture),
                        map(m.metallic_roughness_texture),
                        map(m.emissive_texture),
                        m.transforms.map(|t| (t.scale, t.offset, t.rotation))
                    ),
                    None => log::info!("  pbr {id}: {} (drawn with the default material)", self.materials.describe(&id)),
                }
            }
            for (face, ov) in world.gltf_overrides.get(&o.key).iter().flat_map(|s| s.iter()) {
                log::info!("  pbr override face {face}: {ov:?}");
            }
            for id in mats {
                match self.legacy_mats.peek(&id) {
                    Some(lm) => log::info!(
                        "  material {id}: normal {} x{:.2?} rot {:.2}, specular {} x{:.2?} rot {:.2}, color {:?} exp {} env {} alpha mode {}",
                        lm.normal_map,
                        &lm.normal_st[..2],
                        lm.normal_rot,
                        lm.specular_map,
                        &lm.specular_st[..2],
                        lm.specular_rot,
                        lm.specular_color,
                        lm.specular_exp,
                        lm.env_intensity,
                        lm.diffuse_alpha_mode
                    ),
                    None => log::info!("  material {id}: not loaded"),
                }
            }
        }
        log::info!("nearby objects: {n} within {radius} m of {center:.1?}");
        // faces with glow worn by the avatars around (what feeds the glow pass)
        for (idx, o) in world.objects.iter() {
            let Some(g) = self.gpu.get(idx) else {
                continue;
            };
            if g.owner_avatar.is_none() || g.hud || g.center.distance(center) > radius {
                continue;
            }
            let te = o.te.as_ref();
            for f in &g.faces {
                let fi = f.te_index as usize;
                let tf = te.map(|t| *t.face(fi)).unwrap_or_default();
                if tf.glow <= 0.0 {
                    continue;
                }
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                // the attachment root: the prim whose parent is the avatar
                let mut root = o;
                for _ in 0..256 {
                    match world.objects.parent_of(root).and_then(|p| world.objects.get(p)) {
                        Some(p) if !p.is_avatar() => root = p,
                        _ => break,
                    }
                }
                let item = root
                    .attachment_item_id()
                    .and_then(|id| world.inventory.items.get(&id))
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "?".into());
                log::info!(
                    "  worn glow: '{item}' prim {} face {fi}: glow {:.2}, tex {} ({}, {:?}, alpha channel {}), color {:.2?}, fullbright {}, material {} legacy alpha {:?}, pass {:?}, two sided {}",
                    o.full_id,
                    tf.glow,
                    tf.texture,
                    self.textures.describe(&tf.texture),
                    a,
                    self.textures
                        .alpha_channel_by_slot
                        .get(f.base_slot as usize)
                        .copied()
                        .unwrap_or(false),
                    tf.color,
                    tf.fullbright(),
                    tf.material_id,
                    f.legacy_alpha,
                    classify(f, a),
                    f.two_sided
                );
            }
        }
    }
}
