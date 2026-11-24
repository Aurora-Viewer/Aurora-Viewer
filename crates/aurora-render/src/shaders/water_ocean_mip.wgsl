// The mips of enhanced water's finer wind-sea maps (water_ocean.wgsl).

@group(0) @binding(0) var larger: texture_2d_array<f32>;
@group(0) @binding(1) var smaller: texture_storage_2d_array<rgba16float, write>;

// One mip of every layer: the mean of the four texels under it (slopes
// average into the smoother surface a farther pixel sees).
@compute @workgroup_size(8, 8, 1)
fn cs_mip(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(smaller);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let s = vec2<i32>(id.xy) * 2;
    for (var layer = 0; layer < 5; layer++) {
        let v = textureLoad(larger, s, layer, 0) + textureLoad(larger, s + vec2<i32>(1, 0), layer, 0)
            + textureLoad(larger, s + vec2<i32>(0, 1), layer, 0) + textureLoad(larger, s + vec2<i32>(1, 1), layer, 0);
        textureStore(smaller, vec2<i32>(id.xy), layer, v * 0.25);
    }
}
