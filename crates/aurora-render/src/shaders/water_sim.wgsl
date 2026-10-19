// Bounded damped wave equation. Persistent foam is advected on the same grid.
struct Controls { area: vec4<f32>, previous: vec4<f32>, clock: vec4<f32>, counts: vec4<u32> };
struct Event { position: vec4<f32>, motion: vec4<f32>, params: vec4<f32> };
@group(0) @binding(0) var<uniform> fx: Controls;
@group(0) @binding(1) var<storage, read> events: array<Event>;
@group(0) @binding(2) var old: texture_2d<f32>;
@group(0) @binding(3) var next: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var ground: texture_2d<f32>;

fn previous(p: vec2<i32>, size: i32) -> vec4<f32> {
    if (fx.previous.z == 1.0 || any(p < vec2<i32>(0)) || any(p >= vec2<i32>(size))) { return vec4<f32>(0.0); }
    return textureLoad(old, p, 0);
}
fn bed(uv: vec2<f32>) -> vec2<f32> {
    let p = clamp(uv * 64.0 - 0.5, vec2<f32>(0.0), vec2<f32>(63.0));
    let i = vec2<i32>(floor(p));
    let j = min(i + vec2<i32>(1), vec2<i32>(63));
    let f = fract(p);
    return mix(mix(textureLoad(ground, i, 0).xy, textureLoad(ground, vec2<i32>(j.x, i.y), 0).xy, f.x),
        mix(textureLoad(ground, vec2<i32>(i.x, j.y), 0).xy, textureLoad(ground, j, 0).xy, f.x), f.y);
}
@compute @workgroup_size(8, 8)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = i32(textureDimensions(old).x);
    if (any(id.xy >= vec2<u32>(u32(size)))) { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / f32(size);
    let world = fx.area.xy + uv * fx.area.z;
    let shift = vec2<i32>(round((fx.area.xy - fx.previous.xy) * fx.area.w));
    let p = vec2<i32>(id.xy) + shift;
    let a = previous(p, size);
    let b = bed(uv);
    let depth = b.y - b.x;
    if (depth < -0.15) { textureStore(next, vec2<i32>(id.xy), vec4<f32>(0.0)); return; }
    let dt = fx.previous.w;
    let lap = previous(p + vec2<i32>(1,0), size).x + previous(p - vec2<i32>(1,0), size).x
        + previous(p + vec2<i32>(0,1), size).x + previous(p - vec2<i32>(0,1), size).x - 4.0 * a.x;
    // c <= 0.4 cells / dt: stable even at the largest quality and a slow frame.
    let speed = min(3.0, 0.4 / max(dt * fx.area.w, 0.001));
    var velocity = (a.y + lap * speed * speed * fx.area.w * fx.area.w * dt) * exp(-dt * 1.15);
    // Sub-texel advection keeps foam moving without quantized scrolling.
    let drift=vec2<f32>(0.16,0.07)*dt*fx.area.w;
    let fractional=fract(-drift); let offset=vec2<i32>(floor(-drift));
    let q=p+offset;
    var foam=mix(mix(previous(q,size).z,previous(q+vec2<i32>(1,0),size).z,fractional.x),
        mix(previous(q+vec2<i32>(0,1),size).z,previous(q+vec2<i32>(1,1),size).z,fractional.x),fractional.y)*exp(-dt*0.65);
    if(fx.previous.z>1.5){foam=0.0;}
    var contact_mask=0.0;
    if ((fx.counts.z & 16u) != 0u) {
        for (var i = 0u; i < fx.counts.x; i++) {
            let e = events[i];
            if (abs(e.params.z - b.y) > 0.2) { continue; }
            if(e.params.y==6.0 && (fx.counts.z &4u)!=0u) {
                // The entry's frothy rim expands as the crown collapses. Its
                // wave impulse below is still injected only once, not per frame.
                let age=fx.clock.x-e.position.w;
                if(age>0.0 && age<1.6) {
                    let radius=e.params.x+age*(1.2+e.motion.w*0.3);
                    let width=0.14+age*0.22;
                    let ring=exp(-pow((length(world-e.position.xy)-radius)/max(width,0.55/fx.area.w),2.0));
                    foam+=ring*e.motion.w*exp(-age*2.4)*dt*3.0;
                }
            }
            if (e.position.w <= fx.clock.y) { continue; }
            // Spray and air are presentation only. Their independent switches
            // must never add or remove energy from the displacement field.
            if(e.params.y!=3.0 && e.params.y!=6.0){continue;}
            let depth=max(b.y-e.position.z,0.0);
            if(depth>4.0){continue;}
            let radius = max(e.params.x, max(0.35,0.8/fx.area.w))+depth*0.2;
            let delta=world-e.position.xy;
            let q=length(delta/radius);
            let falloff=exp(-q*q*3.0)*e.motion.w*exp(-depth*1.5);
            if(q<2.0) {
                // Zero-volume impulse: a step makes expanding ripples, not a raised mound.
                velocity+=(1.0-3.0*q*q)*falloff*0.9;
                if((fx.counts.z &4u)!=0u){foam+=falloff*0.35;}
                if(e.params.y==6.0 && (fx.counts.z &4u)!=0u){foam+=exp(-pow((q-0.9)*5.0,2.0))*e.motion.w*1.2;}
            }
            let moving=length(e.motion.xy);
            if(e.params.w>0.0 && moving>0.5) {
                let direction=e.motion.xy/moving;
                let along=dot(delta,direction);let across=dot(delta,vec2<f32>(-direction.y,direction.x));
                let tail=-along-e.params.w*0.35;
                let reach=clamp(moving*1.6+e.params.w,3.0,18.0);
                if(tail>0.0 && tail<reach) {
                    // Diverging wake arms, near the deep-water Kelvin angle; foam follows the stern.
                    let width=radius*0.55+tail*0.354;
                    let arm=exp(-pow((abs(across)-width)/max(0.35,0.65/fx.area.w+tail*0.035),2.0));
                    let fade=(1.0-tail/reach)*smoothstep(0.0,0.8,tail);
                    velocity+=arm*sin(tail*2.4)*fade*e.motion.w*0.28;
                    if((fx.counts.z &4u)!=0u) {
                        let churn=exp(-pow(across/max(radius,0.5),2.0))*exp(-tail*0.35);
                        foam+=(arm*0.55+churn*0.7)*fade*e.motion.w;
                    }
                }
            }
        }
    }
    if ((fx.counts.z & 4u) != 0u) {
        for (var i = fx.counts.x; i < fx.counts.x + fx.counts.y; i++) {
            let e = events[i];
            if(abs(e.params.z-b.y)>0.2){continue;}
            let d = world - e.position.xy;
            let axis = normalize(e.motion.xy + vec2<f32>(0.00001,0.0));
            let q = abs(vec2<f32>(dot(d,axis), dot(d,vec2<f32>(-axis.y,axis.x)))) - e.params.xw;
            let edge = length(max(q,vec2<f32>(0.0))) + min(max(q.x,q.y),0.0);
            let near_edge = exp(-edge * edge * 5.0);
            contact_mask=max(contact_mask,exp(-edge*edge*2.0));
            foam += near_edge * (0.16 + 0.08 * sin(fx.clock.x * 1.6 + dot(world,vec2<f32>(1.4,1.7)))) * dt;
            if (edge < -0.2) { velocity *= 0.5; foam *= 0.5; }
        }
    }
    if ((fx.counts.z & 2u) != 0u) {
        let wash=sin(depth*3.4+fx.clock.x*1.25+sin(world.x*0.4+world.y*0.31)*0.7);
        foam+=exp(-pow((depth-0.3)*2.8,2.0))*pow(max(wash,0.0),4.0)*dt*1.4;
    }
    let border = min(min(uv.x,uv.y),min(1.0-uv.x,1.0-uv.y));
    let fade = smoothstep(0.0,0.04,border) * smoothstep(-0.15,0.2,depth);
    textureStore(next, vec2<i32>(id.xy), vec4<f32>(clamp(a.x + velocity * dt,-0.3,0.3) * fade,
        clamp(velocity,-2.0,2.0) * fade, clamp(foam,0.0,1.0) * fade, contact_mask*fade));
}
