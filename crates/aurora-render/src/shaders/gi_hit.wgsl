// The light at a traced hit with the traced global illumination's volume
// for its ambient (gi.rs `bound_sample_source`, after rt_light.wgsl and
// gi_sample.wgsl): rt_light.wgsl `hit_light`, its sky hemisphere replaced
// where the volume covers the hit. The volume's light has the sky as the
// probes see it, so a hit in a closed room no longer takes the open sky's,
// and the light the faces around bounced, so the indirect light that reads
// it has more than one bounce.
// `ray`: from the ray's origin to the hit.
fn gi_hit_light(lit: HitLight, s: Surface, hit: vec3<f32>, ray: vec3<f32>, offset: f32, reach: f32, posed: bool, lamps: Lamps) -> vec3<f32> {
    let g = gi_irradiance(hit, s.normal, -ray);
    let ambient = mix(hemisphere(lit, s.normal), g.rgb, g.a);
    return hit_light(lit, s, hit, offset, reach, posed, lamps, ambient);
}

// gi_sample.wgsl's visibility of a probe from a point: one shadow ray
// against the scene's faces (`gi_probe_ray`, the includer's: rt_light.wgsl
// `unblocked`, or the same ray counted, shaders/gi_count.wgsl; the posed
// faces, avatars that move, hide no probe). It ends at the first face met,
// and the probes are less than two cells' diagonal from the point.
// `gi_probe_rays` false: every probe is taken for seen, with no ray (the
// diagnosis `AURORA_RT_GI_DIAG` of the indirect light's passes alone).
var<private> gi_probe_rays: bool = true;

fn gi_probe_seen(probe: vec3<f32>, point: vec3<f32>) -> bool {
    if (!gi_probe_rays) {
        return true;
    }
    let d = point - probe;
    let r = length(d);
    if (r < 1e-3) {
        return true;
    }
    return gi_probe_ray(probe, d / r, r);
}

// rt_light.wgsl `hit_radiance` with `gi_hit_light`: the hit's base colour
// (from mip `lod`) under that light, plus its emission.
fn gi_hit_radiance(lit: HitLight, s: Surface, hit: vec3<f32>, ray: vec3<f32>, lod: f32, offset: f32, reach: f32, posed: bool, lamps: Lamps) -> vec3<f32> {
    let base = base_color(s.record, s.uv, lod).rgb;
    return base * gi_hit_light(lit, s, hit, ray, offset, reach, posed, lamps) + emissive_color(s.record);
}
