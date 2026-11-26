@compute @workgroup_size(64) fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    if (id.x >= arrayLength(&probes)) { return; }
    let p = probes[id.x];
    avatar_shadow_receiver = p.normal.w > 0.5;
    let sample = graded_shadow(p.position.xyz, p.normal.xyz, graded_distance(p.position.xyz));
    let combined = combine_sun_visibility(sample.a, p.position.w);
    results[id.x] = vec4<f32>(sample.a, combined, 0.0, 0.0);
}
