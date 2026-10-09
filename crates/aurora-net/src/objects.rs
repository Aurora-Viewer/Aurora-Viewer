//! Decoding of object update messages into plain data.
//!
//! Formats from `LLViewerObject::processUpdateMessage` and
//! `LLViewerObjectList::processCompressedObjectUpdate` (originally LGPL 2.1).

use aurora_msg::msgs;
use aurora_prim::params::{LL_PCODE_LEGACY_AVATAR, RawShape};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams, parse_extra_params, parse_texture_entry};
use glam::{Quat, Vec3, Vec4};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextureAnim {
    pub mode: u8,
    pub face: i8,
    pub size_x: u8,
    pub size_y: u8,
    pub start: f32,
    pub length: f32,
    pub rate: f32,
}

impl TextureAnim {
    pub const ON: u8 = 0x01;
    pub const LOOP: u8 = 0x02;
    pub const REVERSE: u8 = 0x04;
    pub const PING_PONG: u8 = 0x08;
    pub const SMOOTH: u8 = 0x10;
    pub const ROTATE: u8 = 0x20;
    pub const SCALE: u8 = 0x40;

    /// TextureAnim block of a compressed update or of the object cache
    /// (LLTextureAnim::unpackTAMessage(LLDataPacker&): sizes as sent).
    pub fn parse(d: &[u8]) -> Option<TextureAnim> {
        if d.len() != 16 {
            return None;
        }
        let f = |o: usize| f32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        Some(TextureAnim {
            mode: d[0],
            face: d[1] as i8,
            size_x: d[2],
            size_y: d[3],
            start: f(4),
            length: f(8),
            rate: f(12),
        })
    }

    /// TextureAnim block of a full ObjectUpdate
    /// (LLTextureAnim::unpackTAMessage(LLMessageSystem*)): the frame grid is
    /// at least 1 × 1 unless the animation is smooth.
    pub fn parse_message(d: &[u8]) -> Option<TextureAnim> {
        let mut a = Self::parse(d)?;
        if a.mode & Self::SMOOTH == 0 {
            a.size_x = a.size_x.max(1);
            a.size_y = a.size_y.max(1);
        }
        Some(a)
    }
}

/// What an object update says about the object's particle system.
///
/// The raw bytes are parsed viewer-side with
/// `aurora_prim::particles::parse` (`None` there also means "no system").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ParticleUpdate {
    /// Leave the current particle source untouched.
    #[default]
    Keep,
    /// The object has no particle system: kill its source
    /// (`LLViewerObject::deleteParticleSource`; live particles fade out).
    Clear,
    /// New `PSBlock` bytes (legacy 86-byte or size-prefixed new format).
    Set(Vec<u8>),
}

/// A full object description.
/// Sound attached to an object (llLoopSound / llPlaySound), as carried by
/// its updates: a nil id clears it (LLViewerObject::setAttachedSound).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AttachedSound {
    pub id: Uuid,
    pub gain: f32,
    /// LL_SOUND_FLAG_* (loop 1, sync master 2, sync slave 4, queue 16, stop 32).
    pub flags: u8,
    /// Cut-off radius (m, 0 = none: llSetSoundRadius).
    pub radius: f32,
}

#[derive(Debug, Clone)]
pub struct ObjectUpdate {
    pub local_id: u32,
    pub full_id: Uuid,
    pub parent_id: u32,
    pub pcode: u8,
    pub state: u8,
    pub crc: u32,
    pub material: u8,
    pub click_action: u8,
    pub scale: Vec3,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub acceleration: Vec3,
    pub angular_velocity: Vec3,
    pub update_flags: u32,
    pub owner_id: Uuid,
    pub volume: VolumeParams,
    pub texture_entry: Option<Arc<TextureEntry>>,
    pub extra: ExtraParams,
    pub name_values: String,
    pub text: String,
    pub text_color: [u8; 4],
    pub media_url: String,
    /// Tree species (legacy trees/grass).
    pub tree_species: Option<u8>,
    pub texture_anim: Option<TextureAnim>,
    pub foot_plane: Option<Vec4>,
    /// Particle system data carried by this update.
    pub particles: ParticleUpdate,
    pub sound: AttachedSound,
}

impl ObjectUpdate {
    pub fn is_avatar(&self) -> bool {
        self.pcode == LL_PCODE_LEGACY_AVATAR
    }

    /// Parse "FirstName STRING RW SV Foo\nLastName STRING RW SV Bar" pairs.
    pub fn name_value(&self, key: &str) -> Option<String> {
        for line in self.name_values.lines() {
            let mut it = line.splitn(5, ' ');
            let name = it.next()?;
            if name == key {
                let _ty = it.next();
                let _class = it.next();
                let _sendto = it.next();
                return it.next().map(|s| s.trim().to_owned());
            }
        }
        None
    }

    /// Attachment point index from the state byte.
    pub fn attachment_point(&self) -> u8 {
        ((self.state & 0xF0) >> 4) | ((self.state & 0x0F) << 4)
    }
}

/// Movement-only update.
#[derive(Debug, Clone)]
pub struct TerseUpdate {
    pub local_id: u32,
    pub state: u8,
    pub is_avatar: bool,
    pub foot_plane: Option<Vec4>,
    pub position: Vec3,
    pub velocity: Vec3,
    pub acceleration: Vec3,
    pub rotation: Quat,
    pub angular_velocity: Vec3,
    pub texture_entry: Option<Arc<TextureEntry>>,
}

#[inline]
pub fn u16_to_f32(v: u16, lower: f32, upper: f32) -> f32 {
    let delta = upper - lower;
    let mut val = v as f32 / 65535.0 * delta + lower;
    // Snap values that are within one quantum of zero.
    let max_error = delta / 65535.0;
    if val.abs() < max_error {
        val = 0.0;
    }
    val
}

struct Dp<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Dp<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn vec4(&mut self) -> Option<Vec4> {
        Some(Vec4::new(self.f32()?, self.f32()?, self.f32()?, self.f32()?))
    }
    fn uuid(&mut self) -> Option<Uuid> {
        Uuid::from_slice(self.take(16)?).ok()
    }
    /// NUL-terminated string.
    fn cstr(&mut self) -> Option<String> {
        let rest = self.d.get(self.p..)?;
        let end = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.p += end + 1;
        Some(s)
    }
    /// S32 length-prefixed binary.
    fn binary(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn rest(&self) -> &'a [u8] {
        self.d.get(self.p..).unwrap_or(&[])
    }
}

fn sanitize_quat(q: Quat) -> Quat {
    let n = q.normalize();
    if n.is_finite() { n } else { Quat::IDENTITY }
}

fn sanitize_vec(v: Vec3) -> Vec3 {
    if v.is_finite() { v } else { Vec3::ZERO }
}

fn str_of(b: &[u8]) -> String {
    aurora_msg::field_str(b)
}

fn apply_sculpt(volume: &mut VolumeParams, extra: &ExtraParams) {
    volume.sculpt = extra.sculpt;
}

/// Decode a full (uncompressed) `ObjectUpdate` block.
pub fn parse_full(b: &msgs::object_update::ObjectData) -> Option<ObjectUpdate> {
    let mut dp = Dp::new(&b.object_data);
    let mut foot_plane = None;
    let (position, velocity, acceleration, rotation, angular_velocity) = match b.object_data.len() {
        76 | 140 | 60 | 124 => {
            if matches!(b.object_data.len(), 76 | 140) {
                foot_plane = Some(dp.vec4()?);
            }
            let pos = dp.vec3()?;
            let vel = dp.vec3()?;
            let acc = dp.vec3()?;
            let rot = aurora_msg::unpack_quat(dp.vec3()?);
            let angv = dp.vec3()?;
            (pos, vel, acc, rot, angv)
        }
        _ => return None,
    };
    let raw = RawShape {
        path_curve: b.path_curve,
        profile_curve: b.profile_curve,
        path_begin: b.path_begin,
        path_end: b.path_end,
        path_scale_x: b.path_scale_x,
        path_scale_y: b.path_scale_y,
        path_shear_x: b.path_shear_x,
        path_shear_y: b.path_shear_y,
        path_twist: b.path_twist,
        path_twist_begin: b.path_twist_begin,
        path_radius_offset: b.path_radius_offset,
        path_taper_x: b.path_taper_x,
        path_taper_y: b.path_taper_y,
        path_revolutions: b.path_revolutions,
        path_skew: b.path_skew,
        profile_begin: b.profile_begin,
        profile_end: b.profile_end,
        profile_hollow: b.profile_hollow,
    };
    let extra = parse_extra_params(&b.extra_params);
    let mut volume = raw.to_params();
    apply_sculpt(&mut volume, &extra);
    let mut text_color = [0u8; 4];
    if b.text_color.len() == 4 {
        text_color.copy_from_slice(&b.text_color);
        text_color[3] = 255 - text_color[3];
    }
    let tree_species = if matches!(b.p_code, 0xFF | 0x6F | 0x5F) {
        b.data.first().copied()
    } else {
        None
    };
    Some(ObjectUpdate {
        local_id: b.id,
        full_id: b.full_id,
        parent_id: b.parent_id,
        pcode: b.p_code,
        state: b.state,
        crc: b.crc,
        material: b.material,
        click_action: b.click_action,
        scale: sanitize_vec(b.scale),
        position: sanitize_vec(position),
        rotation: sanitize_quat(rotation),
        velocity: sanitize_vec(velocity),
        acceleration: sanitize_vec(acceleration),
        angular_velocity: sanitize_vec(angular_velocity),
        update_flags: b.update_flags,
        owner_id: b.owner_id,
        volume,
        texture_entry: parse_texture_entry(&b.texture_entry).map(Arc::new),
        extra,
        name_values: str_of(&b.name_value),
        text: str_of(&b.text),
        text_color,
        media_url: str_of(&b.media_url),
        tree_species,
        texture_anim: TextureAnim::parse_message(&b.texture_anim),
        foot_plane,
        sound: AttachedSound {
            id: b.sound,
            gain: b.gain,
            flags: b.flags,
            radius: b.radius,
        },
        // Full updates always carry PSBlock; empty means no system.
        particles: if b.ps_block.is_empty() {
            ParticleUpdate::Clear
        } else {
            ParticleUpdate::Set(b.ps_block.clone())
        },
    })
}

/// Decode one `ObjectUpdateCompressed` data blob.
pub fn parse_compressed(data: &[u8], update_flags: u32) -> Option<ObjectUpdate> {
    let mut dp = Dp::new(data);
    let full_id = dp.uuid()?;
    let local_id = dp.u32()?;
    let pcode = dp.u8()?;
    let state = dp.u8()?;
    let crc = dp.u32()?;
    let material = dp.u8()?;
    let click_action = dp.u8()?;
    let scale = dp.vec3()?;
    let position = dp.vec3()?;
    let rotation = aurora_msg::unpack_quat(dp.vec3()?);
    let flags = dp.u32()?;
    let owner_id = dp.uuid()?;

    let mut angular_velocity = Vec3::ZERO;
    if flags & 0x80 != 0 {
        angular_velocity = dp.vec3()?;
    }
    let parent_id = if flags & 0x20 != 0 { dp.u32()? } else { 0 };
    let mut tree_species = None;
    if flags & 0x2 != 0 {
        tree_species = Some(dp.u8()?);
    } else if flags & 0x1 != 0 {
        let _size = dp.u32()?;
        let _ = dp.binary()?;
    }
    let mut text = String::new();
    let mut text_color = [0u8; 4];
    if flags & 0x4 != 0 {
        text = dp.cstr()?;
        let c = dp.take(4)?;
        text_color = [c[0], c[1], c[2], 255 - c[3]];
    }
    let mut media_url = String::new();
    if flags & 0x200 != 0 {
        media_url = dp.cstr()?;
    }
    // Particle system (llviewerobject.cpp, compressed path): 0x8 carries a
    // legacy fixed 86-byte block; 0x400 means new-format particles are
    // present but not sent here, so keep the current source; otherwise the
    // object has no particle system.
    let particles = if flags & 0x8 != 0 {
        ParticleUpdate::Set(dp.take(86)?.to_vec())
    } else if flags & 0x400 != 0 {
        ParticleUpdate::Keep
    } else {
        ParticleUpdate::Clear
    };
    // Extra params: count, then {u16 type, binary}
    let ep_start = dp.p;
    let n = dp.u8()?;
    for _ in 0..n {
        dp.u16()?;
        dp.binary()?;
    }
    let extra = parse_extra_params(&data[ep_start..dp.p]);

    let mut sound = AttachedSound::default();
    if flags & 0x10 != 0 {
        sound = AttachedSound {
            id: dp.uuid()?,
            gain: dp.f32()?,
            flags: dp.u8()?,
            radius: dp.f32()?,
        };
    }
    let mut name_values = String::new();
    if flags & 0x100 != 0 {
        name_values = dp.cstr()?;
    }

    // Volume params (path then profile)
    let raw = RawShape {
        path_curve: dp.u8()?,
        path_begin: dp.u16()?,
        path_end: dp.u16()?,
        path_scale_x: dp.u8()?,
        path_scale_y: dp.u8()?,
        path_shear_x: dp.u8()?,
        path_shear_y: dp.u8()?,
        path_twist: dp.u8()? as i8,
        path_twist_begin: dp.u8()? as i8,
        path_radius_offset: dp.u8()? as i8,
        path_taper_x: dp.u8()? as i8,
        path_taper_y: dp.u8()? as i8,
        path_revolutions: dp.u8()?,
        path_skew: dp.u8()? as i8,
        profile_curve: dp.u8()?,
        profile_begin: dp.u16()?,
        profile_end: dp.u16()?,
        profile_hollow: dp.u16()?,
    };
    let mut volume = raw.to_params();
    apply_sculpt(&mut volume, &extra);

    let te = dp.binary().unwrap_or(&[]);
    let texture_entry = parse_texture_entry(te).map(Arc::new);

    let mut texture_anim = None;
    if flags & 0x40 != 0
        && let Some(ta) = dp.binary()
    {
        texture_anim = TextureAnim::parse(ta);
    }
    let _ = dp.rest();

    Some(ObjectUpdate {
        local_id,
        full_id,
        parent_id,
        pcode,
        state,
        crc,
        material,
        click_action,
        scale: sanitize_vec(scale),
        position: sanitize_vec(position),
        rotation: sanitize_quat(rotation),
        velocity: Vec3::ZERO,
        acceleration: Vec3::ZERO,
        angular_velocity: sanitize_vec(angular_velocity),
        update_flags,
        owner_id,
        volume,
        texture_entry,
        extra,
        name_values,
        text,
        text_color,
        media_url,
        tree_species,
        texture_anim,
        foot_plane: None,
        particles,
        sound,
    })
}

/// Decode an `ImprovedTerseObjectUpdate` block.
pub fn parse_terse(b: &msgs::improved_terse_object_update::ObjectData) -> Option<TerseUpdate> {
    let mut dp = Dp::new(&b.data);
    let local_id = dp.u32()?;
    let state = dp.u8()?;
    let is_avatar = dp.u8()? != 0;
    let foot_plane = if is_avatar { Some(dp.vec4()?) } else { None };
    let position = dp.vec3()?;
    let mut u = [0u16; 13];
    for v in u.iter_mut() {
        *v = dp.u16()?;
    }
    let velocity = Vec3::new(
        u16_to_f32(u[0], -128.0, 128.0),
        u16_to_f32(u[1], -128.0, 128.0),
        u16_to_f32(u[2], -128.0, 128.0),
    );
    let acceleration = Vec3::new(
        u16_to_f32(u[3], -64.0, 64.0),
        u16_to_f32(u[4], -64.0, 64.0),
        u16_to_f32(u[5], -64.0, 64.0),
    );
    let rotation = Quat::from_xyzw(
        u16_to_f32(u[6], -1.0, 1.0),
        u16_to_f32(u[7], -1.0, 1.0),
        u16_to_f32(u[8], -1.0, 1.0),
        u16_to_f32(u[9], -1.0, 1.0),
    );
    let angular_velocity = Vec3::new(
        u16_to_f32(u[10], -64.0, 64.0),
        u16_to_f32(u[11], -64.0, 64.0),
        u16_to_f32(u[12], -64.0, 64.0),
    );
    let texture_entry = if b.texture_entry.len() > 4 {
        // Terse TE is prefixed with its own u32 size.
        parse_texture_entry(&b.texture_entry[4..]).map(Arc::new)
    } else {
        None
    };
    Some(TerseUpdate {
        local_id,
        state,
        is_avatar,
        foot_plane,
        position: sanitize_vec(position),
        velocity: sanitize_vec(velocity),
        acceleration: sanitize_vec(acceleration),
        rotation: sanitize_quat(rotation),
        angular_velocity: sanitize_vec(angular_velocity),
        texture_entry,
    })
}

/// `LLGenericStreamingMessage::METHOD_GLTF_MATERIAL_OVERRIDE`.
pub const METHOD_GLTF_MATERIAL_OVERRIDE: u16 = 0x4175;

/// Faces an override message can address (`MAX_TES` of applyOverrideMessage).
const MAX_OVERRIDE_TES: usize = 45;

/// Payload of a GLTF material override message (LLGenericStreamingMessage::
/// unpack + LLGLTFMaterialList::applyOverrideMessage): LLSD notation
/// `{'id':<local id>,'te':[faces],'od':[override per face]}`. Returns the
/// local id and the (face, override) pairs; `None` for a malformed message
/// (no `te` array), which LL ignores. Faces outside 0..45 are dropped (LL
/// indexes its table with them unchecked).
pub fn parse_gltf_override(payload: &[u8]) -> Option<(u32, Vec<(u8, aurora_llsd::Llsd)>)> {
    // LL copies at most 7 KB (MAX_SIZE)
    let payload = &payload[..payload.len().min(7 * 1024)];
    let data = aurora_llsd::from_notation(payload).ok()?;
    let local_id = data.get("id").as_i32() as u32;
    let tes = data.get("te");
    if !tes.is_array() {
        return None;
    }
    let od = data.get("od");
    let sides = tes
        .as_array()
        .iter()
        .take(MAX_OVERRIDE_TES)
        .enumerate()
        .filter_map(|(i, te)| {
            let face = u8::try_from(te.as_i32()).ok().filter(|&f| (f as usize) < MAX_OVERRIDE_TES)?;
            Some((face, od.at(i).clone()))
        })
        .collect();
    Some((local_id, sides))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gltf_override_message() {
        let (id, sides) = parse_gltf_override(b"{'id':i1234,'te':[i0,i2,i99,i-1],'od':[{'ti':[{'s':[r10,r10]}]},{'mf':r0}]}\0").unwrap();
        assert_eq!(id, 1234);
        assert_eq!(sides.len(), 2, "out of range faces are dropped");
        assert_eq!(sides[0].0, 0);
        assert_eq!(sides[0].1.get("ti").at(0).get("s").at(0).as_f64(), 10.0);
        assert_eq!(sides[1].0, 2);
        let (_, cleared) = parse_gltf_override(b"{'id':i7,'te':[],'od':[]}").unwrap();
        assert!(cleared.is_empty());
        assert!(parse_gltf_override(b"{'id':i7}").is_none());
        assert!(parse_gltf_override(b"garbage").is_none());
    }

    #[test]
    fn texture_anim_blocks() {
        // ANIM_ON | LOOP, all sides, 0 × 0 grid, start 1, length 2, rate -0.5
        let mut d = vec![0x03, 0xFF, 0, 0];
        for v in [1.0f32, 2.0, -0.5] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        let a = TextureAnim::parse(&d).expect("16 bytes");
        assert_eq!((a.mode, a.face, a.size_x, a.size_y), (3, -1, 0, 0));
        assert_eq!((a.start, a.length, a.rate), (1.0, 2.0, -0.5));
        let m = TextureAnim::parse_message(&d).expect("16 bytes");
        assert_eq!((m.size_x, m.size_y), (1, 1));
        d[0] |= TextureAnim::SMOOTH;
        let m = TextureAnim::parse_message(&d).expect("16 bytes");
        assert_eq!((m.size_x, m.size_y), (0, 0));
        assert!(TextureAnim::parse(&d[..15]).is_none());
    }

    #[test]
    fn u16_quant() {
        assert_eq!(u16_to_f32(32767, -1.0, 1.0), 0.0);
        assert!((u16_to_f32(65535, -1.0, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn compressed_garbage_is_safe() {
        for n in 0..200 {
            let d: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(7)).collect();
            let _ = parse_compressed(&d, 0);
        }
    }

    #[test]
    fn name_values() {
        let mut o = parse_compressed(&[0u8; 0], 0);
        assert!(o.is_none());
        o = Some(ObjectUpdate {
            local_id: 1,
            full_id: Uuid::nil(),
            parent_id: 0,
            pcode: 47,
            state: 0,
            crc: 0,
            material: 0,
            click_action: 0,
            scale: Vec3::ONE,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            update_flags: 0,
            owner_id: Uuid::nil(),
            volume: Default::default(),
            texture_entry: None,
            extra: Default::default(),
            name_values: "FirstName STRING RW SV Jane\nLastName STRING RW SV Resident\nTitle STRING RW SV Group Name".into(),
            text: String::new(),
            text_color: [0; 4],
            media_url: String::new(),
            tree_species: None,
            texture_anim: None,
            foot_plane: None,
            particles: ParticleUpdate::Keep,
            sound: Default::default(),
        });
        let o = o.unwrap();
        assert_eq!(o.name_value("FirstName").as_deref(), Some("Jane"));
        assert_eq!(o.name_value("Title").as_deref(), Some("Group Name"));
    }
}
