// Forward ray projection: overlapping refracted triangles add light, conserving
// their source area. Three receiving depths share RGB in one bounded light map.
struct CausticControls { area:vec4<f32>, previous:vec4<f32>, clock:vec4<f32>, counts:vec4<u32> };
@group(0) @binding(0) var<uniform> caustic_fx:CausticControls;
struct CausticOut {
    @builtin(position) clip:vec4<f32>,
    @location(0) source:vec2<f32>,
    @location(1) @interpolate(flat) layer:u32,
};
@vertex
fn vs_caustic(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->CausticOut {
    let sizes=array<u32,4>(96u,160u,224u,256u);
    let n=sizes[min(caustic_fx.counts.w,3u)];let count=n*n;
    let layer=instance/count;let cell=instance%count;
    let corners=array<vec2<f32>,6>(vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,1.0),vec2<f32>(0.0,1.0));
    let p=(vec2<f32>(f32(cell%n),f32(cell/n))+corners[vertex])/f32(n)*18.0-1.0;
    let wave=water_micro(p,caustic_fx.clock.x,caustic_fx.counts.w,0.0);
    let normal=normalize(vec3<f32>(-wave.xy,1.0));
    let ray=refract(vec3<f32>(0.0,0.0,-1.0),normal,0.7501875);
    let depths=array<f32,3>(0.75,2.5,7.0);
    let hit=p+ray.xy*(depths[layer]+wave.z)/max(-ray.z,0.2);
    var out:CausticOut;out.clip=vec4<f32>(hit.x*0.125-1.0,1.0-hit.y*0.125,0.0,1.0);
    out.source=p;out.layer=layer;return out;
}
@fragment
fn fs_caustic(in:CausticOut)->@location(0) vec4<f32> {
    let dx=dpdx(in.source);let dy=dpdy(in.source);
    let area=abs(dx.x*dy.y-dx.y*dy.x);
    let depths=array<f32,3>(0.75,2.5,7.0);
    let curvature=water_micro_curvature(in.source,caustic_fx.clock.x,caustic_fx.counts.w)*depths[in.layer]*0.25;
    let jacobian=(1.0+curvature.x)*(1.0+curvature.z)-curvature.y*curvature.y;
    // The analytic lens Jacobian smooths tessellation plateaus; projected area
    // anchors flux. A finite solar disc / pixel footprint bounds singular folds.
    let focus=inverseSqrt(jacobian*jacobian+0.015);
    let intensity=min(mix(area*256.0,focus,0.85),6.0);
    return vec4<f32>(select(0.0,intensity,in.layer==0u),select(0.0,intensity,in.layer==1u),select(0.0,intensity,in.layer==2u),0.0);
}
