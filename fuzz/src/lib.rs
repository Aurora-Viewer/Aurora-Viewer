//! The fuzz targets: each feeds arbitrary bytes to a reader of data that a
//! resident or a simulator controls (assets, UDP messages, LLSD), the way
//! the viewer calls it. A target only has to return: any panic, endless
//! loop or oversized allocation is a bug of the reader. `seeds` gives each
//! target valid inputs made by our own encoders (or shipped assets), so the
//! mutations start from the formats' real structure; nothing comes from the
//! grid cache.

use aurora_llsd::Llsd;
use aurora_msg::{Msg, msgs};

pub struct Target {
    pub name: &'static str,
    pub run: fn(&[u8]),
    pub seeds: fn() -> Vec<Vec<u8>>,
}

pub const TARGETS: &[Target] = &[
    Target {
        name: "j2k",
        run: j2k,
        seeds: j2k_seeds,
    },
    Target {
        name: "mesh",
        run: mesh,
        seeds: mesh_seeds,
    },
    Target {
        name: "anim",
        run: anim,
        seeds: anim_seeds,
    },
    Target {
        name: "llsd",
        run: llsd,
        seeds: llsd_seeds,
    },
    Target {
        name: "material",
        run: material,
        seeds: material_seeds,
    },
    Target {
        name: "packet",
        run: packet,
        seeds: packet_seeds,
    },
    Target {
        name: "object",
        run: object,
        seeds: object_seeds,
    },
    Target {
        name: "prim",
        run: prim,
        seeds: prim_seeds,
    },
    Target {
        name: "volume",
        run: volume,
        seeds: volume_seeds,
    },
    Target {
        name: "sculpt",
        run: sculpt,
        seeds: sculpt_seeds,
    },
    Target {
        name: "terrain",
        run: terrain,
        seeds: terrain_seeds,
    },
    Target {
        name: "sound",
        run: sound,
        seeds: sound_seeds,
    },
    Target {
        name: "misc",
        run: misc,
        seeds: misc_seeds,
    },
];

pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

/// The entry point of the libFuzzer binaries.
pub fn run(name: &str, data: &[u8]) {
    if let Some(t) = target(name) {
        (t.run)(data);
    }
}

// ---- textures: JPEG2000 codestreams (discard level from the first byte)

fn j2k(data: &[u8]) {
    let Some((&discard, body)) = data.split_first() else { return };
    if let Some(info) = aurora_assets::j2k_info(body) {
        let _ = aurora_assets::bytes_for_discard(&info, discard % 6);
    }
    let _ = aurora_assets::decode_j2k(body, discard % 6);
}

fn j2k_seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (w, h, c) in [(8u32, 8u32, 3u8), (16, 8, 4), (4, 4, 1), (33, 17, 3)] {
        let pixels: Vec<u8> = (0..w * h * u32::from(c)).map(|i| (i * 37 % 251) as u8).collect();
        for lossless in [false, true] {
            if let Ok(bytes) = aurora_assets::j2k::encode_texture_j2k(w, h, c, &pixels, lossless) {
                out.push([&[0u8][..], &bytes].concat());
            }
        }
    }
    out
}

// ---- meshes: the header, every section it names, and raw sections

fn mesh(data: &[u8]) {
    if let Ok(header) = aurora_assets::parse_mesh_header(data) {
        let _ = header.actual_lod(3);
        for range in header.lods.into_iter().flatten() {
            if let Some(section) = aurora_assets::mesh::section_bytes(data, range) {
                let _ = aurora_assets::decode_mesh_lod(section);
            }
        }
        if let Some(section) = header.skin.and_then(|range| aurora_assets::mesh::section_bytes(data, range)) {
            let _ = aurora_assets::decode_skin(section);
        }
    }
    let _ = aurora_assets::decode_mesh_lod(data);
    let _ = aurora_assets::decode_skin(data);
}

fn mesh_seeds() -> Vec<Vec<u8>> {
    use aurora_assets::import_mesh::{Face, Model, Skin, Vertex, encode, process};
    use glam::{Mat4, Vec3};
    let cube = |skinned: bool| {
        let vertices = [-0.5f32, 0.5]
            .into_iter()
            .flat_map(|x| {
                [-0.5f32, 0.5].into_iter().flat_map(move |y| {
                    [-0.5f32, 0.5].into_iter().map(move |z| Vertex {
                        position: [x, y, z],
                        normal: [0.0, 0.0, 1.0],
                        uv: [x + 0.5, y + 0.5],
                        joints: [0; 4],
                        weights: if skinned { [1.0, 0.0, 0.0, 0.0] } else { [0.0; 4] },
                    })
                })
            })
            .collect();
        Model {
            name: "Cube".into(),
            submodel: 0,
            center: Vec3::ZERO,
            size: Vec3::ONE,
            skin: skinned.then(|| Skin {
                names: vec!["mPelvis".into()],
                inverse_bind: vec![Mat4::IDENTITY],
                alternate_bind: vec![Mat4::IDENTITY],
                bind_shape: Mat4::IDENTITY,
                pelvis_offset: 0.0,
            }),
            faces: vec![Face {
                material: 0,
                vertices,
                indices: vec![
                    0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6, 2, 1, 3, 7, 1, 7, 5,
                ],
            }],
        }
    };
    let mut out = Vec::new();
    for skinned in [false, true] {
        let model = cube(skinned);
        let options = encode::Options {
            skin: skinned,
            joints: skinned,
            ..encode::Options::default()
        };
        let lods = [Some(model.clone()), None, Some(model.clone()), Some(model)];
        if let Ok(asset) = encode::asset(&lods, None, &process::Physics::default(), options) {
            if let Ok(header) = aurora_assets::parse_mesh_header(&asset) {
                for range in header.lods.into_iter().flatten().chain(header.skin) {
                    if let Some(section) = aurora_assets::mesh::section_bytes(&asset, range) {
                        out.push(section.to_vec());
                    }
                }
            }
            out.push(asset);
        }
    }
    out
}

// ---- animations (`.anim` keyframe assets)

fn anim(data: &[u8]) {
    let _ = aurora_assets::parse_animation(data);
    let _ = aurora_assets::parse_animation_upload(data);
}

const SKELETON: &[u8] = include_bytes!("../../crates/aurora-viewer/assets/character/avatar_skeleton.xml");
const CANNONBALL: &[u8] = include_bytes!("../../crates/aurora-viewer/assets/animations/swimming/cannonball.anim");
const BVH: &[u8] = include_bytes!("../../crates/aurora-viewer/assets/animations/swimming/backstroke.bvh");

fn anim_seeds() -> Vec<Vec<u8>> {
    let mut out = vec![CANNONBALL.to_vec()];
    if let Ok(skeleton) = aurora_assets::Skeleton::parse(SKELETON)
        && let Ok(bvh) = aurora_assets::import_animation::parse_bvh(BVH, &skeleton)
        && let Ok(bytes) = aurora_assets::import_animation::encode(&bvh.animation)
    {
        out.push(bytes);
    }
    out
}

// ---- LLSD in its three encodings, and XML-RPC replies (login)

fn llsd(data: &[u8]) {
    let _ = aurora_llsd::from_binary(data);
    let _ = aurora_llsd::from_notation(data);
    let _ = aurora_llsd::from_xml(data);
    let _ = aurora_net::xmlrpc::parse_response(data);
}

fn sample_llsd() -> Llsd {
    let mut map = aurora_llsd::Map::new();
    map.insert("name".to_owned(), Llsd::String("Example <&> \u{e9}".into()));
    map.insert("count".to_owned(), Llsd::Integer(-42));
    map.insert("ratio".to_owned(), Llsd::Real(0.5));
    map.insert("ok".to_owned(), Llsd::Boolean(true));
    map.insert(
        "id".to_owned(),
        Llsd::Uuid(uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0)),
    );
    map.insert("bytes".to_owned(), Llsd::Binary(vec![0, 1, 2, 255]));
    map.insert(
        "list".to_owned(),
        Llsd::Array(vec![
            Llsd::Undef,
            Llsd::Date(1.7e9),
            Llsd::Uri("https://example.com/a?b".into()),
            Llsd::Integer(7),
            Llsd::String(String::new()),
        ]),
    );
    Llsd::Map(map)
}

fn llsd_seeds() -> Vec<Vec<u8>> {
    let value = sample_llsd();
    vec![
        aurora_llsd::to_binary(&value),
        aurora_llsd::to_notation(&value).into_bytes(),
        aurora_llsd::to_xml(&value),
        br#"<?xml version="1.0"?><methodResponse><params><param><value><struct><member><name>login</name><value><string>true</string></value></member><member><name>circuit_code</name><value><i4>123</i4></value></member><member><name>buddy-list</name><value><array><data><value><struct><member><name>buddy_id</name><value><string>00000000-0000-0000-0000-000000000001</string></value></member></struct></value></data></array></value></member></struct></value></param></params></methodResponse>"#.to_vec(),
    ]
}

// ---- GLTF materials: assets and the per-face overrides of objects

fn material(data: &[u8]) {
    let _ = aurora_assets::parse_material_asset(data);
    let _ = aurora_net::objects::parse_gltf_override(data);
}

fn material_seeds() -> Vec<Vec<u8>> {
    let gltf = r#"{"asset":{"version":"2.0"},"materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0.5,0.25,1],"metallicFactor":0.2,"roughnessFactor":0.7,"baseColorTexture":{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.1,0.2],"scale":[2,2],"rotation":0.5}}}},"alphaMode":"MASK","alphaCutoff":0.4,"doubleSided":true,"emissiveFactor":[0.1,0.1,0.1]}],"textures":[{"source":0}],"images":[{"uri":"00000000-0000-0000-0000-000000000001"}]}"#;
    let mut map = aurora_llsd::Map::new();
    map.insert("version".to_owned(), Llsd::String("1.1".into()));
    map.insert("type".to_owned(), Llsd::String("GLTF 2.0".into()));
    map.insert("data".to_owned(), Llsd::String(gltf.into()));
    let asset = Llsd::Map(map);
    vec![
        aurora_llsd::to_binary(&asset),
        aurora_llsd::to_xml(&asset),
        aurora_llsd::to_notation(&asset).into_bytes(),
    ]
}

// ---- UDP packets: framing, zerocoding, and the object / terrain messages

fn packet(data: &[u8]) {
    let Ok(packet) = aurora_msg::parse_packet(data) else { return };
    match packet.id {
        msgs::ObjectUpdate::ID => {
            if let Ok(m) = packet.decode::<msgs::ObjectUpdate>() {
                for b in &m.object_data {
                    let _ = aurora_net::objects::parse_full(b);
                }
            }
        }
        msgs::ObjectUpdateCompressed::ID => {
            if let Ok(m) = packet.decode::<msgs::ObjectUpdateCompressed>() {
                for b in &m.object_data {
                    let _ = aurora_net::objects::parse_compressed(&b.data, b.update_flags);
                }
            }
        }
        msgs::ImprovedTerseObjectUpdate::ID => {
            if let Ok(m) = packet.decode::<msgs::ImprovedTerseObjectUpdate>() {
                for b in &m.object_data {
                    let _ = aurora_net::objects::parse_terse(b);
                }
            }
        }
        msgs::LayerData::ID => {
            if let Ok(m) = packet.decode::<msgs::LayerData>() {
                let _ = aurora_net::terrain::decode_land_layer(m.layer_id.type_, &m.layer_data.data);
                let _ = aurora_net::terrain::decode_wind_layer(m.layer_id.type_, &m.layer_data.data);
            }
        }
        _ => {}
    }
}

fn packet_seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // (build_packet returns the number of acks it appended; the datagram is the buffer)
    let mut push = |bytes: &mut Vec<u8>| out.push(std::mem::take(bytes));
    let mut buf = Vec::new();
    let full = msgs::ObjectUpdate {
        region_data: msgs::object_update::RegionData {
            region_handle: 1 << 40,
            time_dilation: 65535,
        },
        object_data: vec![msgs::object_update::ObjectData {
            id: 7,
            full_id: uuid::Uuid::from_u128(7),
            p_code: 9,
            scale: glam::Vec3::splat(0.5),
            object_data: vec![0; 60],
            path_curve: 16,
            profile_curve: 1,
            path_scale_x: 100,
            path_scale_y: 100,
            texture_entry: prim_seeds().remove(0),
            ..Default::default()
        }],
    };
    aurora_msg::build_packet(&full, 1, true, false, &[], &mut buf);
    push(&mut buf);
    let compressed = msgs::ObjectUpdateCompressed {
        region_data: msgs::object_update_compressed::RegionData::default(),
        object_data: object_seeds()
            .into_iter()
            .map(|data| msgs::object_update_compressed::ObjectData {
                update_flags: 0,
                data: data[4..].to_vec(),
            })
            .collect(),
    };
    aurora_msg::build_packet(&compressed, 2, false, false, &[1, 2], &mut buf);
    push(&mut buf);
    let terse = msgs::ImprovedTerseObjectUpdate {
        region_data: msgs::improved_terse_object_update::RegionData::default(),
        object_data: vec![msgs::improved_terse_object_update::ObjectData {
            data: vec![0; 60],
            texture_entry: Vec::new(),
        }],
    };
    aurora_msg::build_packet(&terse, 3, false, true, &[], &mut buf);
    push(&mut buf);
    for seed in terrain_seeds() {
        let layer = msgs::LayerData {
            layer_id: msgs::layer_data::LayerID { type_: seed[0] },
            layer_data: msgs::layer_data::LayerData { data: seed[1..].to_vec() },
        };
        aurora_msg::build_packet(&layer, 4, false, false, &[], &mut buf);
        push(&mut buf);
    }
    out
}

// ---- objects: compressed updates (flags in the first four bytes) and
// texture animations

fn object(data: &[u8]) {
    let Some((flags, body)) = data.split_first_chunk::<4>() else {
        return;
    };
    let _ = aurora_net::objects::parse_compressed(body, u32::from_le_bytes(*flags));
    let _ = aurora_net::objects::TextureAnim::parse(body);
    let _ = aurora_net::objects::TextureAnim::parse_message(body);
}

fn object_seeds() -> Vec<Vec<u8>> {
    // The blob read by parse_compressed (LLViewerObject::processUpdateMessage,
    // OUT_FULL_COMPRESSED), behind the target's four flag bytes, once with
    // no optional block and once with every one of them.
    let te = prim_seeds().remove(0);
    let blob = |flags: u32| {
        let mut b = Vec::new();
        b.extend(uuid::Uuid::from_u128(5).as_bytes());
        b.extend(5u32.to_le_bytes());
        b.extend([9u8, 0]);
        b.extend(0u32.to_le_bytes());
        b.extend([3u8, 0]);
        for v in [0.5f32, 0.5, 0.5, 128.0, 128.0, 25.0, 0.0, 0.0, 0.0] {
            b.extend(v.to_le_bytes());
        }
        b.extend(flags.to_le_bytes());
        b.extend(uuid::Uuid::from_u128(6).as_bytes());
        if flags & 0x80 != 0 {
            for v in [0.0f32, 0.0, 1.0] {
                b.extend(v.to_le_bytes());
            }
        }
        if flags & 0x20 != 0 {
            b.extend(4u32.to_le_bytes());
        }
        if flags & 0x2 != 0 {
            b.push(3);
        }
        if flags & 0x4 != 0 {
            b.extend(b"Hover text\0");
            b.extend([255, 255, 255, 0]);
        }
        if flags & 0x200 != 0 {
            b.extend(b"https://example.com/\0");
        }
        if flags & 0x8 != 0 {
            b.extend(prim_seeds().remove(2));
        }
        let extra = prim_seeds().remove(1);
        b.extend(&extra);
        if flags & 0x10 != 0 {
            b.extend(uuid::Uuid::from_u128(7).as_bytes());
            b.extend(1.0f32.to_le_bytes());
            b.push(1);
            b.extend(20.0f32.to_le_bytes());
        }
        if flags & 0x100 != 0 {
            b.extend(b"AttachItemID STRING RW SV 00000000-0000-0000-0000-000000000008\0");
        }
        // path then profile parameters (a box)
        b.extend([16u8, 0, 0, 0, 0, 100, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        b.extend((te.len() as u32).to_le_bytes());
        b.extend(&te);
        if flags & 0x40 != 0 {
            b.extend(16u32.to_le_bytes());
            b.extend([0x03, 0, 4, 4]);
            for v in [0.0f32, 16.0, 2.0] {
                b.extend(v.to_le_bytes());
            }
        }
        [&0u32.to_le_bytes()[..], &b].concat()
    };
    vec![blob(0), blob(0x80 | 0x20 | 0x2 | 0x4 | 0x200 | 0x8 | 0x10 | 0x100 | 0x40)]
}
// ---- primitive blocks: texture entries, extra parameters, particles

fn prim(data: &[u8]) {
    let _ = aurora_prim::parse_texture_entry(data);
    let _ = aurora_prim::parse_extra_params(data);
    let _ = aurora_prim::particles::parse(data);
}

fn prim_seeds() -> Vec<Vec<u8>> {
    // A texture entry: default texture, a face mask exception, then each
    // field's default value and terminator, as LLPrimitive::packTEMessage.
    let mut te = Vec::new();
    te.extend(uuid::Uuid::from_u128(1).as_bytes());
    te.push(0x81);
    te.push(0x02);
    te.extend(uuid::Uuid::from_u128(2).as_bytes());
    te.push(0);
    te.extend([255, 255, 255, 0, 0]);
    for v in [1.0f32, 1.0] {
        te.extend(v.to_le_bytes());
        te.push(0);
    }
    te.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // Extra parameters: a flexible block and a light block.
    let mut extra = vec![2];
    extra.extend(0x10u16.to_le_bytes());
    extra.extend(16u32.to_le_bytes());
    extra.extend([
        0x22, 0x11, 0x05, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    extra.extend(0x20u16.to_le_bytes());
    extra.extend(16u32.to_le_bytes());
    extra.extend([255, 128, 64, 255]);
    for v in [10.0f32, 0.5, 0.75] {
        extra.extend(v.to_le_bytes());
    }
    // A particle system block (LLPartSysData::pack, legacy 86 bytes).
    let mut particles = vec![0u8; 86];
    particles[0] = 1;
    vec![te, extra, particles]
}

// ---- prim shapes from the object messages' quantized parameters

fn raw_shape(d: &[u8]) -> Option<(aurora_prim::RawShape, &[u8])> {
    let (b, rest) = d.split_first_chunk::<23>()?;
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let shape = aurora_prim::RawShape {
        path_curve: b[0],
        profile_curve: b[1],
        path_begin: u16_at(2),
        path_end: u16_at(4),
        path_scale_x: b[6],
        path_scale_y: b[7],
        path_shear_x: b[8],
        path_shear_y: b[9],
        path_twist: b[10] as i8,
        path_twist_begin: b[11] as i8,
        path_radius_offset: b[12] as i8,
        path_taper_x: b[13] as i8,
        path_taper_y: b[14] as i8,
        path_revolutions: b[15],
        path_skew: b[16] as i8,
        profile_begin: u16_at(17),
        profile_end: u16_at(19),
        profile_hollow: u16_at(21),
    };
    Some((shape, rest))
}

/// The detail of a level of detail, or an arbitrary one from four bytes.
fn detail(rest: &[u8]) -> f32 {
    match rest {
        [] => 1.0,
        [b, ..] if b % 2 == 0 => [0.5f32, 1.0, 2.0, 4.0][usize::from(b / 2 % 4)],
        [_, tail @ ..] => tail.first_chunk::<4>().map_or(1.0, |f| f32::from_le_bytes(*f)),
    }
}

fn volume(data: &[u8]) {
    let Some((shape, rest)) = raw_shape(data) else { return };
    let params = shape.to_params();
    let _ = aurora_prim::generate_volume(&params, detail(rest));
    let _ = aurora_prim::lod_triangle_counts(&params);
    let _ = aurora_prim::num_faces(&params);
}

fn volume_seeds() -> Vec<Vec<u8>> {
    // box, cylinder, sphere, torus, tube, ring, prism
    [(16u8, 1u8), (16, 0), (32, 5), (32, 0), (48, 1), (48, 3), (16, 3)]
        .into_iter()
        .map(|(path, profile)| {
            let mut s = vec![path, profile, 0, 0, 0, 0, 100, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            s.extend([2, 0, 0, 0, 0]);
            s
        })
        .collect()
}

// ---- sculpted prims from a decoded sculpt map of any size

fn sculpt(data: &[u8]) {
    let Some((shape, rest)) = raw_shape(data) else { return };
    let Some((head, pixels)) = rest.split_first_chunk::<6>() else {
        return;
    };
    let mut params = shape.to_params();
    params.sculpt = Some(aurora_prim::SculptParams {
        texture: uuid::Uuid::from_u128(1),
        sculpt_type: head[0],
    });
    // Sculpt maps are textures: at most 2048 a side.
    let width = u32::from(u16::from_le_bytes([head[1], head[2]])) % 2049;
    let height = u32::from(u16::from_le_bytes([head[3], head[4]])) % 2049;
    let _ = aurora_prim::generate_sculpt(&params, detail(&head[5..]), width, height, head[5], pixels);
    let _ = aurora_prim::sculpt_placeholder(&params, 1.0);
}

fn sculpt_seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (kind, size, comps) in [(1u8, 8u16, 3u8), (2, 16, 4), (3, 4, 3), (4, 8, 1), (5, 8, 3)] {
        let mut s = vec![16u8, 1, 0, 0, 0, 0, 100, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        s.push(kind);
        s.extend(size.to_le_bytes());
        s.extend(size.to_le_bytes());
        s.push(comps);
        s.extend((0..u32::from(size) * u32::from(size) * u32::from(comps)).map(|i| (i * 7 % 256) as u8));
        out.push(s);
    }
    out
}

// ---- terrain and wind patches (layer type in the first byte)

fn terrain(data: &[u8]) {
    let Some((&kind, body)) = data.split_first() else { return };
    let _ = aurora_net::terrain::decode_land_layer(kind, body);
    let _ = aurora_net::terrain::decode_wind_layer(kind, body);
}

fn terrain_seeds() -> Vec<Vec<u8>> {
    // LayerData of type L (land) / 7 (wind): group header (stride, patch
    // size, type), then a patch header and its end-of-patch code.
    let mut out = Vec::new();
    for kind in [b'L', b'M', b'7', b'8', b'1'] {
        let mut s = vec![kind];
        s.extend(264u16.to_le_bytes());
        s.push(16);
        s.push(kind);
        s.extend([0x6E, 0x00, 0x00, 0x00, 0x40, 0x10, 0x00, 0x00, 0x00, 0x00]);
        s.extend([0xA2, 0x00]);
        out.push(s);
    }
    out
}

// ---- sounds (Ogg Vorbis assets; the decoder also reads WAV)

fn sound(data: &[u8]) {
    let _ = aurora_audio::SoundClip::decode(data);
}

fn sound_seeds() -> Vec<Vec<u8>> {
    let samples: Vec<i16> = (0..400).map(|i| ((i as f32 * 0.2).sin() * 8000.0) as i16).collect();
    let data_len = (samples.len() * 2) as u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + data_len).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(11025u32.to_le_bytes());
    wav.extend(22050u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_len.to_le_bytes());
    for s in samples {
        wav.extend(s.to_le_bytes());
    }
    vec![wav]
}

// ---- text and small binary assets: notecards, task inventories,
// gestures, wearables, avatar base meshes

fn misc(data: &[u8]) {
    let _ = aurora_net::inventory::notecard::Notecard::decode(data);
    let _ = aurora_net::task_inventory::parse_legacy(data);
    let _ = aurora_assets::gesture::Gesture::decode(data);
    let _ = aurora_assets::wearable::Wearable::parse(data);
    let _ = aurora_assets::parse_llm(data);
}

const EYE: &[u8] = include_bytes!("../../crates/aurora-viewer/assets/character/avatar_eye.llm");

fn misc_seeds() -> Vec<Vec<u8>> {
    vec![
        b"Linden text version 2\n{\nLLEmbeddedItems version 1\n{\ncount 0\n}\nText length 12\nHello world!}\n".to_vec(),
        b"\tinv_object\t0\n\t{\n\t\tobj_id\t00000000-0000-0000-0000-000000000001\n\t\tparent_id\t00000000-0000-0000-0000-000000000000\n\t\ttype\tcategory\n\t\tname\tContents|\n\t}\n\tinv_item\t0\n\t{\n\t\titem_id\t00000000-0000-0000-0000-000000000002\n\t\tparent_id\t00000000-0000-0000-0000-000000000001\n\tpermissions 0\n\t{\n\t\tbase_mask\t7fffffff\n\t\towner_mask\t7fffffff\n\t\tgroup_mask\t00000000\n\t\teveryone_mask\t00000000\n\t\tnext_owner_mask\t00082000\n\t\tcreator_id\t00000000-0000-0000-0000-000000000003\n\t\towner_id\t00000000-0000-0000-0000-000000000003\n\t}\n\t\tasset_id\t00000000-0000-0000-0000-000000000004\n\t\ttype\tnotecard\n\t\tinv_type\tnotecard\n\t\tflags\t00000000\n\tsale_info\t0\n\t{\n\t\tsale_type\tnot\n\t\tsale_price\t10\n\t}\n\t\tname\tNote|\n\t\tdesc\t|\n\t\tcreation_date\t1700000000\n\t}\n".to_vec(),
        b"LLWearable version 22\nShirt\n\n\tpermissions 0\n\t{\n\t\tbase_mask\t7fffffff\n\t}\n\tsale_info\t0\n\t{\n\t\tsale_type\tnot\n\t\tsale_price\t10\n\t}\ntype 4\nparameters 2\n781 0.5\n800 1\ntextures 1\n4 00000000-0000-0000-0000-000000000005\n".to_vec(),
        // A gesture (LLMultiGesture v2): an animation, a chat line and a wait.
        b"2\n255\n0\n/hello\nhi there\n3\n0\nwave\n00000000-0000-0000-0000-000000000009\n0\n2\nhello\n0\n3\n1.5\n1\n".to_vec(),
        EYE.to_vec(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every target runs its seeds; the seeds that our own encoders made are
    /// read back by the readers they are made for.
    #[test]
    fn every_target_runs_its_seeds() {
        for t in TARGETS {
            let seeds = (t.seeds)();
            assert!(!seeds.is_empty(), "{} has no seed", t.name);
            for seed in &seeds {
                (t.run)(seed);
            }
        }
    }

    /// Seeds the readers reject would leave the mutations far from the
    /// formats' structure.
    #[test]
    fn the_seeds_are_valid_inputs() {
        let j2k = j2k_seeds();
        assert!(j2k.len() >= 4, "{} j2k seeds", j2k.len());
        for s in &j2k {
            aurora_assets::decode_j2k(&s[1..], 0).expect("j2k seed");
        }
        let mesh = mesh_seeds();
        let assets: Vec<_> = mesh.iter().filter(|s| aurora_assets::parse_mesh_header(s).is_ok()).collect();
        assert_eq!(assets.len(), 2, "two mesh assets");
        assert!(mesh.iter().any(|s| aurora_assets::decode_skin(s).is_ok()), "a skin section");
        assert!(mesh.iter().any(|s| aurora_assets::decode_mesh_lod(s).is_ok()), "a LOD section");
        for s in anim_seeds() {
            aurora_assets::parse_animation(&s).expect("anim seed");
        }
        assert_eq!(anim_seeds().len(), 2);
        let llsd = llsd_seeds();
        let value = sample_llsd();
        assert_eq!(aurora_llsd::from_binary(&llsd[0]).expect("binary").0, value);
        assert_eq!(aurora_llsd::from_notation(&llsd[1]).expect("notation"), value);
        assert_eq!(aurora_llsd::from_xml(&llsd[2]).expect("xml"), value);
        aurora_net::xmlrpc::parse_response(&llsd[3]).expect("xml-rpc");
        for s in material_seeds() {
            aurora_assets::parse_material_asset(&s).expect("material seed");
        }
        for s in packet_seeds() {
            aurora_msg::parse_packet(&s).expect("packet seed");
        }
        let te = &prim_seeds()[0];
        assert!(aurora_prim::parse_texture_entry(te).is_some(), "texture entry");
        let object = object_seeds();
        assert!(
            aurora_net::objects::parse_compressed(&object[0][4..], 0).is_some(),
            "compressed update"
        );
        for s in volume_seeds() {
            let (shape, _) = raw_shape(&s).expect("shape");
            assert!(aurora_prim::num_faces(&shape.to_params()) > 0);
        }
        assert!(
            sound(&sound_seeds()[0]) == () && aurora_audio::SoundClip::decode(&sound_seeds()[0]).is_ok(),
            "wav"
        );
        let misc = misc_seeds();
        aurora_net::inventory::notecard::Notecard::decode(&misc[0]).expect("notecard");
        assert_eq!(aurora_net::task_inventory::parse_legacy(&misc[1]).expect("task inventory").len(), 2);
        aurora_assets::wearable::Wearable::parse(&misc[2]).expect("wearable");
        aurora_assets::gesture::Gesture::decode(&misc[3]).expect("gesture");
        aurora_assets::parse_llm(&misc[4]).expect("llm");
    }
}
