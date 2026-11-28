// Windows' DLSS frame generation (dlss_fg.rs `Interface`): the interface
// alone, for Streamline's "UI colour and alpha" (DLSS-G guide, UI
// handling). A pixel the interface drew on differs between the frame as
// shown and the frame before the interface (the rest are the same image,
// copied twice): there the interface is the shown pixel, opaque; elsewhere
// nothing, alpha 0. Premultiplied, as the guide asks, and shown = interface
// + (1 - alpha) * hudless holds at every pixel.

@group(0) @binding(0) var shown: texture_2d<f32>;
@group(0) @binding(1) var hudless: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(p.xy);
    let real = textureLoad(shown, at, 0);
    if any(real.rgb != textureLoad(hudless, at, 0).rgb) {
        return vec4<f32>(real.rgb, 1.0);
    }
    return vec4<f32>(0.0);
}
