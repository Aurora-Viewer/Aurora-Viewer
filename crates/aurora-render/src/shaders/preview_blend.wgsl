// The see-through faces of a studio in their exact order, as the world draws
// its own (`RiggedBlendDepth::Exact`, "exact blend order" of object.wgsl,
// whose functions come before these): every fragment lit once and filed in the
// front of its pixel, then each pixel composed. Read with preview.wgsl.

// What the opaque faces left, sample by sample.
@group(2) @binding(0) var solid_depth: DEPTH_TEXTURE;

// How much of the pixel the opaque faces let see at `z`, at the middle of the
// pixel, changing by `slope` from one to the next.
fn in_front(clip: vec4<f32>, z: f32, slope: vec2<f32>) -> f32 {
    // (where the four samples of a pixel are)
    var places = array<vec2<f32>,4>(vec2(-0.125,-0.375), vec2(0.375,-0.125), vec2(-0.375,0.125), vec2(0.125,0.375));
    var seen = 0.0;
    for (var i = 0; i < SAMPLES; i++) {
        var at = z;
        if SAMPLES > 1 { at += dot(slope, places[i]); }
        // (what lies on the skin is over it: layers of make-up)
        if at <= textureLoad(solid_depth, vec2<i32>(clip.xy), i) + 0.00000025 { seen += 1.0; }
    }
    return seen / f32(SAMPLES);
}

struct Once {
    @location(0) haze: vec4<f32>,
    @location(1) range: vec4<f32>,
}

@fragment fn fs_once(v: Vertex, @builtin(front_facing) front: bool) -> Once {
    var out: Once;
    out.haze = vec4(0.0);
    out.range = vec4(0.0);
    let m = maps(v);
    let slope = vec2(dpdx(v.clip.z), dpdy(v.clip.z));
    if !front && u.surface.y < 0.5 { discard; return out; }
    let base_color = map_at(base, m, 0) * u.color;
    var cover = base_color.a;
    // (an empty texel, as most are)
    if cover < 0.004 { discard; return out; }
    cover = clamp(cover, 0.0, 1.0) * in_front(v.clip, v.clip.z, slope);
    let a8 = u32(cover * 255.0 + 0.5);
    if a8 == 0u { discard; return out; }
    cover = f32(a8) / 255.0;
    let light = blend_finite(max(shade(v, front, m, base_color), vec3(0.0)) * cover);
    let pixel = blend_pixel(v.clip);
    let head = pixel * 4u;
    // nearer is greater (the world's depth: one over the distance)
    let place = blend_at(clamp(v.clip.w * 0.01, 0.0, 1.0));
    let key = place << 8u;
    if a8 > BLEND_FAINT {
        let rank = atomicAdd(&blend_front[head + BLEND_HEAD_COUNT], 1u) & BLEND_COUNT_MASK;
        if rank < BLEND_SLOTS {
            let at = blend_slot(pixel, rank);
            atomicStore(&blend_front[at + blend_slot_light(rank)], blend_pack(light));
            atomicStore(&blend_front[at], key | a8);
            discard; return out;
        }
        let page = blend_page(head, rank / BLEND_SLOTS - 1u, rank % BLEND_SLOTS == 0u);
        if page != 0u {
            let slot = rank % BLEND_SLOTS;
            atomicStore(&blend_front[page + BLEND_PAGE_LIGHTS + slot], blend_pack(light));
            atomicStore(&blend_front[page + slot], key | a8);
            discard; return out;
        }
    }
    let stops = blend_stops(cover);
    out.haze = vec4(min(light * stops / cover, vec3(HDR_MAX)), 0.0);
    out.range = vec4(f32(place >> blend_order.shift), f32((0x00ffffffu - place) >> blend_order.shift), 0.0, stops);
    return out;
}

// The place of a depth, as `blend_place` without the rank of a layer.
fn blend_at(z: f32) -> u32 {
    let bits = bitcast<u32>(z);
    let e = clamp(bits >> 23u, 96u, 127u) - 96u;
    let m = select((bits >> 7u) & 0xffffu, 0u, (bits >> 23u) < 96u);
    return (e << 19u) | (m << 3u);
}
