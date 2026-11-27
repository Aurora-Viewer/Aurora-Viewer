// The sea grid's shore map (water_shore.rs), reduced to the grid's cells.
// The map is what stands in the water seen from below (object.wgsl
// `fs_shore`): per texel the height of the nearest face over the map's
// lowest (x) and whether it faces up (y 1: the view began inside a solid
// that crosses the water) or down (y 0: an underside, a deck or a floor);
// y -1 where nothing is. Per cell: the share of its texels inside a solid
// and the lowest top seen from inside, the share under an underside and
// the lowest underside (world heights; NONE where there is none).

struct ShoreParams {
    // x the map's lowest height (world z), y texels a cell, z cells a side
    range: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: ShoreParams;
@group(0) @binding(1) var map: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> cells: array<vec4<f32>>;

// water_shore.rs NONE
const NONE: f32 = 1.0e9;

@compute @workgroup_size(8, 8, 1)
fn cs_shore_cells(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = u32(params.range.z);
    let t = u32(params.range.y);
    if (id.x >= n || id.y >= n) {
        return;
    }
    var inside = 0u;
    var top = NONE;
    var under = 0u;
    var low = NONE;
    // (the map's rows run from north to south, as a picture of the ground
    // seen from above; the grid's cells from south to north)
    let row = (n - 1u - id.y) * t;
    for (var y = 0u; y < t; y++) {
        for (var x = 0u; x < t; x++) {
            let v = textureLoad(map, vec2<u32>(id.x * t + x, row + y), 0);
            if (v.y < -0.5) {
                continue;
            }
            if (v.y > 0.5) {
                inside++;
                top = min(top, v.x);
            } else {
                under++;
                low = min(low, v.x);
            }
        }
    }
    let count = f32(t * t);
    let base = params.range.x;
    cells[id.y * n + id.x] = vec4<f32>(
        f32(inside) / count,
        select(NONE, top + base, inside > 0u),
        f32(under) / count,
        select(NONE, low + base, under > 0u),
    );
}
