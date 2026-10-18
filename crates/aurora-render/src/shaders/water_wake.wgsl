// Swept body segments are rasterized into the fine half of the water atlas.
// Cost follows the area touched by a wake, with no per-surface-pixel event loop.
struct Controls { area: vec4<f32>, previous: vec4<f32>, clock: vec4<f32>, counts: vec4<u32> };
struct Event { position: vec4<f32>, motion: vec4<f32>, params: vec4<f32> };
@group(0) @binding(0) var<uniform> fx: Controls;
@group(0) @binding(1) var<storage, read> events: array<Event>;
@group(0) @binding(2) var ground: texture_2d<f32>;
struct WakeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) @interpolate(flat) event: u32,
};
@vertex
fn vs_wake(@builtin(vertex_index) vertex:u32, @builtin(instance_index) instance:u32)->WakeOut {
    let e=events[instance];
    let corners=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(-1.0,1.0),
        vec2<f32>(-1.0,1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0));
    let speed=max(length(e.motion.xy),0.01);
    let direction=e.motion.xy/speed;
    let age=max(fx.clock.x-e.position.w,0.0);
    let width=clamp(e.params.x,max(0.035,32.0/(fx.area.w*fx.area.z)*0.65),0.45);
    let sigma=max(e.params.w*0.7,0.25);
    // The back of the longitudinal reconstruction kernel is older than its
    // endpoint. Bound those spreading shoulders too, avoiding cut-off crests.
    let oldest=min(age+(sigma*3.0+e.params.w*0.5)/speed,2.0);
    let side=width+oldest*(0.4+sqrt(width*9.81)*0.15)+3.0*(width*0.55+oldest*0.09);
    let local=corners[vertex]*vec2<f32>(sigma*3.0,side);
    var p=e.position.xy-direction*e.params.w*0.5+direction*local.x+vec2<f32>(-direction.y,direction.x)*local.y;
    if(e.params.y==8.0) {
        let ring=width+age*(0.55+e.motion.w*0.5);
        p=e.position.xy+corners[vertex]*(ring+3.0*(width*0.55+age*0.09));
    }
    let origin=fx.clock.zw;
    let uv=(p-origin)/32.0;
    var out:WakeOut;out.world=p;out.event=instance;
    out.clip=vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0);
    if((e.params.y!=4.0 && e.params.y!=8.0) || (e.params.y==4.0 && e.params.w<=0.0) || age>=2.0){out.clip=vec4<f32>(2.0,2.0,0.0,1.0);}
    return out;
}
fn wake_bed(p:vec2<f32>)->vec2<f32> {
    let cell=clamp((p-fx.area.xy)/fx.area.z*64.0-0.5,vec2<f32>(0.0),vec2<f32>(63.0));
    let i=vec2<i32>(floor(cell));let j=min(i+vec2<i32>(1),vec2<i32>(63));let f=fract(cell);
    return mix(mix(textureLoad(ground,i,0).xy,textureLoad(ground,vec2<i32>(j.x,i.y),0).xy,f.x),
        mix(textureLoad(ground,vec2<i32>(i.x,j.y),0).xy,textureLoad(ground,j,0).xy,f.x),f.y);
}
@fragment
fn fs_wake(in:WakeOut)->@location(0) vec4<f32> {
    let e=events[in.event];let speed=max(length(e.motion.xy),0.01);
    let direction=e.motion.xy/speed;
    let delta=in.world-e.position.xy+direction*e.params.w*0.5;
    let along=dot(delta,direction);let across=dot(delta,vec2<f32>(-direction.y,direction.x));
    let age=max(fx.clock.x-e.position.w+(e.params.w*0.5-along)/speed,0.0);
    let width=clamp(e.params.x,max(0.035,32.0/(fx.area.w*fx.area.z)*0.65),0.45);
    let sigma=max(e.params.w*0.7,0.25);
    // Unit integral along the path: changing frame rate/emission spacing cannot
    // pile up a taller wall of water. Neighbouring intervals reconstruct a ribbon.
    let weight=exp(-pow(along/sigma,2.0))*e.params.w/(sigma*1.772454);
    let bed=wake_bed(in.world);
    let wet=smoothstep(-0.02,0.25,e.params.z-bed.x)*(1.0-smoothstep(0.05,0.2,abs(bed.y-e.params.z)));
    let strength=e.motion.w*exp(-max(e.params.z-e.position.z,0.0)*1.8);
    let amplitude=strength*0.16*smoothstep(0.12,0.6,speed)*wet;
    let spread=0.4+sqrt(width*9.81)*0.15;
    let shoulder=width+age*spread;
    let ridge_width=width*0.55+age*0.09;
    let x=(abs(across)-shoulder)/ridge_width;
    let core=exp(-pow(across/(width+age*0.18),2.0))*exp(-age*1.5);
    let ridge=exp(-x*x)*(1.0-0.55*x*x)*exp(-age*0.85);
    let trailing=(abs(across)-shoulder*0.58)/(ridge_width*0.75);
    let train=exp(-trailing*trailing)*(1.0-2.0*trailing*trailing)*0.18*smoothstep(0.15,0.65,age);
    let fade=1.0-smoothstep(1.3,2.0,age);
    var height=(ridge*0.85-core+train)*weight*amplitude*fade;
    var foam=exp(-x*x*2.0)*weight*strength*smoothstep(0.15,0.7,speed)*exp(-age*1.7)*fade*wet;
    if(e.params.y==8.0) {
        let age=max(fx.clock.x-e.position.w,0.0);
        let radius=width+age*(0.55+e.motion.w*0.5);
        let q=(length(in.world-e.position.xy)-radius)/(width*0.55+age*0.09);
        let pulse=(1.0-2.0*q*q)*exp(-q*q);
        let fade=(1.0-smoothstep(1.1,1.8,age))*exp(-age*1.7);
        let footfall=e.params.w>0.5;
        height=pulse*strength*select(0.14,0.065,footfall)*fade*wet;
        foam=exp(-q*q*2.0)*strength*select(1.0,0.65,footfall)*exp(-age*2.5)*fade*wet;
    }
    // The atlas viewport maps increasing world X/Y to increasing pixel X/Y.
    let texel=32.0/(fx.area.w*fx.area.z);
    let gradient=vec2<f32>(dpdx(height),dpdy(height))/max(texel,0.001);
    foam*=select(0.0,1.0,(fx.counts.z & 4u)!=0u);
    return vec4<f32>(gradient,height,foam);
}
