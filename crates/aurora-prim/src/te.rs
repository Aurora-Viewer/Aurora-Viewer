//! Texture entries (`TextureEntry` blob), ported from
//! `LLPrimitive::parseTEMessage` / `unpack_TEField`.

use uuid::Uuid;

pub const MAX_TES: usize = 45;
const TEXTURE_ROTATION_PACK_FACTOR: f32 = 32768.0;

/// Default "plywood" texture used when a face has no texture.
pub const DEFAULT_TEXTURE: Uuid = Uuid::from_u128(0x89556747_24cb_43ed_920b_47caed15465f);
/// Fully transparent texture.
pub const TRANSPARENT_TEXTURE: Uuid = Uuid::from_u128(0x8dcd4a48_2d37_4909_9f78_f7a9eb4ef903);
/// Blank (white) texture.
pub const BLANK_TEXTURE: Uuid = Uuid::from_u128(0x5748decc_f629_461c_9a36_a35a221fe21f);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureFace {
    pub texture: Uuid,
    /// Linear RGBA in 0..1.
    pub color: [f32; 4],
    pub scale_s: f32,
    pub scale_t: f32,
    pub offset_s: f32,
    pub offset_t: f32,
    /// Radians.
    pub rotation: f32,
    /// bit 0..4 bump, 5 fullbright, 6..7 shiny (LLTextureEntry TEM_*)
    pub bump_shiny_fullbright: u8,
    /// bit 0 media, 1..2 texgen (0 default, 2 planar)
    pub media_flags: u8,
    pub glow: f32,
    pub material_id: Uuid,
}

impl Default for TextureFace {
    fn default() -> Self {
        Self {
            texture: DEFAULT_TEXTURE,
            color: [1.0; 4],
            scale_s: 1.0,
            scale_t: 1.0,
            offset_s: 0.0,
            offset_t: 0.0,
            rotation: 0.0,
            bump_shiny_fullbright: 0,
            media_flags: 0,
            glow: 0.0,
            material_id: Uuid::nil(),
        }
    }
}

impl TextureFace {
    pub fn fullbright(&self) -> bool {
        self.bump_shiny_fullbright & 0x20 != 0
    }
    pub fn shiny(&self) -> u8 {
        (self.bump_shiny_fullbright >> 6) & 0x3
    }
    pub fn bump(&self) -> u8 {
        self.bump_shiny_fullbright & 0x1f
    }
    pub fn planar(&self) -> bool {
        (self.media_flags >> 1) & 0x3 == 1
    }
}

/// Parsed texture entry: one face record per possible face.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureEntry {
    pub faces: Vec<TextureFace>,
}

impl Default for TextureEntry {
    fn default() -> Self {
        Self {
            faces: vec![TextureFace::default(); MAX_TES],
        }
    }
}

impl TextureEntry {
    pub fn face(&self, i: usize) -> &TextureFace {
        self.faces.get(i).or_else(|| self.faces.first()).unwrap_or(&DEFAULT_FACE)
    }
}

static DEFAULT_FACE: TextureFace = TextureFace {
    texture: DEFAULT_TEXTURE,
    color: [1.0; 4],
    scale_s: 1.0,
    scale_t: 1.0,
    offset_s: 0.0,
    offset_t: 0.0,
    rotation: 0.0,
    bump_shiny_fullbright: 0,
    media_flags: 0,
    glow: 0.0,
    material_id: Uuid::nil(),
};

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// Reads one TE field: default value, then (bitfield, value) pairs until a
/// zero bitfield. Returns `None` if the buffer is malformed.
fn unpack_field<const N: usize>(c: &mut Cursor, out: &mut [[u8; N]; MAX_TES]) -> Option<()> {
    // +1: there must be at least the terminating byte after the value.
    if c.remaining() < N + 1 {
        c.pos = c.data.len();
        return None;
    }
    let mut def = [0u8; N];
    def.copy_from_slice(&c.data[c.pos..c.pos + N]);
    c.pos += N;
    for o in out.iter_mut() {
        *o = def;
    }
    while c.pos < c.data.len() {
        let mut flags: u64 = 0;
        loop {
            let b = *c.data.get(c.pos)?;
            c.pos += 1;
            flags = (flags << 7) | (b & 0x7f) as u64;
            if b & 0x80 == 0 {
                break;
            }
        }
        if flags == 0 {
            break;
        }
        if c.remaining() < N + 1 {
            c.pos = c.data.len();
            return None;
        }
        let mut v = [0u8; N];
        v.copy_from_slice(&c.data[c.pos..c.pos + N]);
        c.pos += N;
        for (i, o) in out.iter_mut().enumerate() {
            if flags & (1u64 << i) != 0 {
                *o = v;
            }
        }
    }
    Some(())
}

/// Parse a TextureEntry blob. Returns `None` on malformed data.
pub fn parse_texture_entry(data: &[u8]) -> Option<TextureEntry> {
    if data.is_empty() {
        return None;
    }
    // The last field isn't zero-terminated; LL appends a 0 byte.
    let mut buf = Vec::with_capacity(data.len() + 1);
    buf.extend_from_slice(&data[..data.len().min(4095)]);
    buf.push(0);
    let mut c = Cursor { data: &buf, pos: 0 };

    let mut images = [[0u8; 16]; MAX_TES];
    let mut colors = [[0u8; 4]; MAX_TES];
    let mut scale_s = [[0u8; 4]; MAX_TES];
    let mut scale_t = [[0u8; 4]; MAX_TES];
    let mut off_s = [[0u8; 2]; MAX_TES];
    let mut off_t = [[0u8; 2]; MAX_TES];
    let mut rot = [[0u8; 2]; MAX_TES];
    let mut bump = [[0u8; 1]; MAX_TES];
    let mut media = [[0u8; 1]; MAX_TES];
    let mut glow = [[0u8; 1]; MAX_TES];
    let mut mats = [[0u8; 16]; MAX_TES];

    unpack_field(&mut c, &mut images)?;
    unpack_field(&mut c, &mut colors)?;
    unpack_field(&mut c, &mut scale_s)?;
    unpack_field(&mut c, &mut scale_t)?;
    unpack_field(&mut c, &mut off_s)?;
    unpack_field(&mut c, &mut off_t)?;
    unpack_field(&mut c, &mut rot)?;
    unpack_field(&mut c, &mut bump)?;
    unpack_field(&mut c, &mut media)?;
    unpack_field(&mut c, &mut glow)?;
    if c.pos >= c.data.len() || unpack_field(&mut c, &mut mats).is_none() {
        mats = [[0u8; 16]; MAX_TES];
    }

    let mut faces = Vec::with_capacity(MAX_TES);
    for i in 0..MAX_TES {
        let col = colors[i];
        let f = TextureFace {
            texture: Uuid::from_bytes(images[i]),
            color: [
                (255 - col[0]) as f32 / 255.0,
                (255 - col[1]) as f32 / 255.0,
                (255 - col[2]) as f32 / 255.0,
                (255 - col[3]) as f32 / 255.0,
            ],
            scale_s: f32::from_le_bytes(scale_s[i]),
            scale_t: f32::from_le_bytes(scale_t[i]),
            offset_s: i16::from_le_bytes(off_s[i]) as f32 / 32767.0,
            offset_t: i16::from_le_bytes(off_t[i]) as f32 / 32767.0,
            rotation: i16::from_le_bytes(rot[i]) as f32 / TEXTURE_ROTATION_PACK_FACTOR * std::f32::consts::TAU,
            bump_shiny_fullbright: bump[i][0],
            media_flags: media[i][0],
            glow: glow[i][0] as f32 / 255.0,
            material_id: Uuid::from_bytes(mats[i]),
        };
        faces.push(f);
    }
    Some(TextureEntry { faces })
}

/// Write one TE field like LLPrimitive::packTEField: the last face's value
/// is the default, then each other distinct value (from the end) with the
/// bitfield of the faces that have it.
fn pack_field<const N: usize>(out: &mut Vec<u8>, values: &[[u8; N]]) {
    let Some(last) = values.len().checked_sub(1) else {
        return;
    };
    out.extend_from_slice(&values[last]);
    for face in (0..last).rev() {
        if values[face + 1..=last].contains(&values[face]) {
            continue; // already sent with a later face
        }
        let mut bits: u64 = 0;
        for i in (0..=face).rev() {
            if values[i] == values[face] {
                bits |= 1 << i;
            }
        }
        // big-endian groups of 7 bits, high bit = more to come
        let mut groups = Vec::with_capacity(7);
        let mut b = bits;
        groups.push((b & 0x7f) as u8);
        b >>= 7;
        while b != 0 {
            groups.push((b & 0x7f) as u8 | 0x80);
            b >>= 7;
        }
        out.extend(groups.iter().rev());
        out.extend_from_slice(&values[face]);
    }
}

/// Serialize the first `num_faces` faces for ObjectImage / ObjectUpdate
/// (LLPrimitive::packTEMessage, indra/llprimitive/llprimitive.cpp,
/// originally LGPL 2.1). The last field has no terminating zero.
pub fn pack_texture_entry(te: &TextureEntry, num_faces: usize) -> Vec<u8> {
    let n = num_faces.min(MAX_TES).min(te.faces.len());
    let faces = &te.faces[..n];
    let mut out = Vec::with_capacity(64 + n * 8);
    if n == 0 {
        return out;
    }
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let fields_end = |out: &mut Vec<u8>| out.push(0);
    pack_field(&mut out, &faces.iter().map(|f| *f.texture.as_bytes()).collect::<Vec<_>>());
    fields_end(&mut out);
    // white (the common color) is sent as zeros: 255 - byte
    pack_field(&mut out, &faces.iter().map(|f| f.color.map(|c| 255 - to_u8(c))).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| f.scale_s.to_le_bytes()).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| f.scale_t.to_le_bytes()).collect::<Vec<_>>());
    fields_end(&mut out);
    let offset = |v: f32| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
    pack_field(&mut out, &faces.iter().map(|f| offset(f.offset_s)).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| offset(f.offset_t)).collect::<Vec<_>>());
    fields_end(&mut out);
    let tau = std::f32::consts::TAU;
    let rot = |r: f32| (((r % tau) / tau * TEXTURE_ROTATION_PACK_FACTOR).round() as i16).to_le_bytes();
    pack_field(&mut out, &faces.iter().map(|f| rot(f.rotation)).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| [f.bump_shiny_fullbright]).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| [f.media_flags]).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| [to_u8(f.glow)]).collect::<Vec<_>>());
    fields_end(&mut out);
    pack_field(&mut out, &faces.iter().map(|f| *f.material_id.as_bytes()).collect::<Vec<_>>());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips() {
        let mut te = TextureEntry::default();
        te.faces[0].texture = Uuid::from_bytes([3; 16]);
        te.faces[2].texture = Uuid::from_bytes([3; 16]);
        te.faces[1].color = [1.0, 0.0, 0.5, 1.0];
        te.faces[3].scale_s = 4.0;
        te.faces[4].offset_t = -0.25;
        te.faces[5].rotation = 1.5;
        te.faces[6].bump_shiny_fullbright = 0x21;
        te.faces[7].media_flags = 0x02;
        te.faces[7].glow = 0.4;
        te.faces[7].material_id = Uuid::from_bytes([9; 16]);
        let n = 8;
        let packed = pack_texture_entry(&te, n);
        let back = parse_texture_entry(&packed).expect("parses");
        for i in 0..n {
            let (a, b) = (&te.faces[i], &back.faces[i]);
            assert_eq!(a.texture, b.texture, "face {i}");
            for c in 0..4 {
                assert!((a.color[c] - b.color[c]).abs() < 0.003);
            }
            assert_eq!(a.scale_s, b.scale_s);
            assert!((a.offset_t - b.offset_t).abs() < 1e-4);
            assert!((a.rotation - b.rotation).abs() < 1e-3);
            assert_eq!(a.bump_shiny_fullbright, b.bump_shiny_fullbright);
            assert_eq!(a.media_flags, b.media_flags);
            assert!((a.glow - b.glow).abs() < 0.003);
            assert_eq!(a.material_id, b.material_id);
        }
        // the default is the last face's value: faces 0 and 2 are one exception
        assert_eq!(&packed[..16], &DEFAULT_TEXTURE.as_bytes()[..]);
        assert_eq!(packed[16], 0b101);
    }

    #[test]
    fn pack_wide_bitfields() {
        let mut te = TextureEntry::default();
        te.faces[0].scale_t = 2.0;
        te.faces[40].scale_t = 2.0;
        let back = parse_texture_entry(&pack_texture_entry(&te, 45)).expect("parses");
        assert_eq!(back.faces[0].scale_t, 2.0);
        assert_eq!(back.faces[40].scale_t, 2.0);
        assert_eq!(back.faces[20].scale_t, 1.0);
    }

    fn build(default_img: [u8; 16], face2_img: [u8; 16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&default_img);
        v.push(0x04); // face 2
        v.extend_from_slice(&face2_img);
        v.push(0); // end images
        v.extend_from_slice(&[0, 0, 0, 0]); // color (inverted => white)
        v.push(0);
        v.extend_from_slice(&1.0f32.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&2.0f32.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&0i16.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&0i16.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&0i16.to_le_bytes());
        v.push(0);
        v.push(0x20); // fullbright (TEM_FULLBRIGHT_SHIFT 5)
        v.push(0);
        v.push(0);
        v.push(0);
        v.push(255); // glow, last field (no terminator)
        v
    }

    #[test]
    fn parses_exceptions() {
        let te = parse_texture_entry(&build([1; 16], [2; 16])).unwrap();
        assert_eq!(te.faces[0].texture, Uuid::from_bytes([1; 16]));
        assert_eq!(te.faces[2].texture, Uuid::from_bytes([2; 16]));
        assert_eq!(te.faces[1].color, [1.0; 4]);
        assert_eq!(te.faces[5].scale_t, 2.0);
        assert!(te.faces[3].fullbright());
        assert!((te.faces[0].glow - 1.0).abs() < 1e-6);
    }

    #[test]
    fn garbage_is_safe() {
        for n in 0..80 {
            let d: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(97).wrapping_add(13)).collect();
            let _ = parse_texture_entry(&d);
        }
    }
}
