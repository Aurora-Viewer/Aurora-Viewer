// Shared by the visible surface and the refracted-light projection.
// Metre/second units; periodic micro-waves tile over 16 m without drifting seams.
struct WaterSwell { offset: vec3<f32>, gradient: vec2<f32>, compression: f32 };
fn water_swell(p: vec2<f32>, time: f32, quality: u32) -> WaterSwell {
    let directions = array<vec2<f32>,4>(vec2<f32>(0.88,0.475),vec2<f32>(0.62,0.785),vec2<f32>(-0.22,0.975),vec2<f32>(0.97,-0.243));
    let frequencies = array<f32,4>(0.19,0.32,0.48,0.71);
    let amplitudes = array<f32,4>(0.24,0.115,0.055,0.025);
    var offset=vec3<f32>(0.0);var gradient=vec2<f32>(0.0);
    var jacobian=vec3<f32>(1.0,0.0,1.0);
    for(var i=0u;i<min(quality+2u,4u);i++) {
        let d=directions[i];let k=frequencies[i];let a=amplitudes[i];
        let phase=dot(p,d)*k-time*sqrt(9.81*k)+f32(i)*1.73;
        let s=sin(phase);let c=cos(phase);
        offset+=vec3<f32>(d*c*a*0.85,s*a);
        gradient+=d*c*a*k;
        jacobian-=a*k*0.85*s*vec3<f32>(d.x*d.x,d.x*d.y,d.y*d.y);
    }
    let determinant=max(jacobian.x*jacobian.z-jacobian.y*jacobian.y,0.25);
    var out:WaterSwell;out.offset=offset;
    out.gradient=vec2<f32>(jacobian.z*gradient.x-jacobian.y*gradient.y,jacobian.x*gradient.y-jacobian.y*gradient.x)/determinant;
    out.compression=1.0-determinant;
    return out;
}
const WATER_MICRO_MODES=array<vec2<f32>,6>(vec2<f32>(12.0,3.0),vec2<f32>(-5.0,17.0),vec2<f32>(19.0,8.0),vec2<f32>(-11.0,23.0),vec2<f32>(25.0,-7.0),vec2<f32>(5.0,31.0));
const WATER_MICRO_AMPLITUDES=array<f32,6>(0.012,0.009,0.0065,0.0045,0.003,0.002);
// xy slope, z height. The same normals bend the rays in the caustics map.
fn water_micro(p: vec2<f32>, time: f32, quality: u32, footprint: f32) -> vec3<f32> {
    var wave=vec3<f32>(0.0);
    for(var i=0u;i<min(quality+4u,6u);i++) {
        let k=WATER_MICRO_MODES[i]*0.3926990817;let frequency=length(k);
        let phase=dot(k,p)-time*sqrt(9.81*frequency)+f32(i)*2.31;
        let attenuation=1.0-smoothstep(0.6,2.5,frequency*footprint);
        wave+=vec3<f32>(k*cos(phase),sin(phase))*WATER_MICRO_AMPLITUDES[i]*attenuation;
    }
    return wave;
}

// xx, xy, yy second derivatives of the same surface, for its optical Jacobian.
fn water_micro_curvature(p:vec2<f32>,time:f32,quality:u32)->vec3<f32> {
    var curvature=vec3<f32>(0.0);
    for(var i=0u;i<min(quality+4u,6u);i++) {
        let k=WATER_MICRO_MODES[i]*0.3926990817;
        let phase=dot(k,p)-time*sqrt(9.81*length(k))+f32(i)*2.31;
        curvature-=vec3<f32>(k.x*k.x,k.x*k.y,k.y*k.y)*sin(phase)*WATER_MICRO_AMPLITUDES[i];
    }
    return curvature;
}
