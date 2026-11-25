// `AURORA_RT_GI_DIAG=count`'s hooks of gi_sample.wgsl and gi_hit.wgsl, in
// rt_gi.wgsl's passes in the place of gi.rs `PLAIN_HOOKS` (rt_gi_counts.rs
// puts `GI_COUNT_SLOTS`, `GI_COUNT_PASSES` and `GI_COUNT_KINDS` before it).
// Each invocation adds up its reads' work in its own counters, then adds
// them to `gi_counts` once, at the end of its pass: `GI_COUNT_SLOTS` copies
// of each pass's `GI_COUNT_KINDS` counters, a workgroup's chosen by its
// place, so that the atomics of the whole screen do not all wait on the
// same words. The probes' visibility rays are the same rays as
// rt_light.wgsl `unblocked` (the scene's faces), with their candidate hits
// counted: those the traversal hands the shader (the faces of a geometry
// that is not opaque), and among them the alpha-masked ones, whose texture
// `stops` reads. A diagnosis: the counts cost time of their own.

@group(0) @binding(24) var<storage, read_write> gi_counts: array<atomic<u32>>;

// gi_sample.wgsl's `GI_NOTE_*` go to 9; the rays' candidates, and the
// alpha-masked ones among them.
const GI_NOTE_CANDIDATE: u32 = 10u;
const GI_NOTE_MASKED: u32 = 11u;

var<private> gi_count_at: u32 = 0u;
var<private> gi_count_n: array<u32, GI_COUNT_KINDS>;

fn gi_count_start(pass_index: u32, group: vec3<u32>) {
    let slot = (group.x * 7u + group.y * 13u) % GI_COUNT_SLOTS;
    gi_count_at = (slot * GI_COUNT_PASSES + pass_index) * GI_COUNT_KINDS;
}

fn gi_note(kind: u32) {
    gi_count_n[kind] += 1u;
}

fn gi_note_add(kind: u32, n: u32) {
    gi_count_n[kind] += n;
}

fn gi_count_flush() {
    for (var k = 0u; k < GI_COUNT_KINDS; k++) {
        if (gi_count_n[k] != 0u) {
            atomicAdd(&gi_counts[gi_count_at + k], gi_count_n[k]);
        }
    }
}

fn gi_probe_ray(origin: vec3<f32>, dir: vec3<f32>, reach: f32) -> bool {
    let ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | SHADOW_CULL, SOLID, 0.0, reach, origin, dir);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        let c = rayQueryGetCandidateIntersection(&rq);
        gi_note(GI_NOTE_CANDIDATE);
        if (c.kind == RAY_QUERY_INTERSECTION_TRIANGLE) {
            let h = hit_of(false, c.instance_custom_data, c.geometry_index, c.primitive_index);
            if ((record_vec(h.record, RECORD_FLAGS).x & FLAG_ALPHA_MASK) != 0u) {
                gi_note(GI_NOTE_MASKED);
            }
        }
        if (stops(c, false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    return rayQueryGetCommittedIntersection(&rq).kind == RAY_QUERY_INTERSECTION_NONE;
}
