struct Eye { world: mat4x4<f32>, fov: vec4<f32>, padding: vec4<f32> }
@group(0) @binding(0) var<uniform> eye: Eye;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
 let p = array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
 var o: V; o.p=vec4(p[i],0.,1.); o.uv=p[i]*0.5+0.5; return o;
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
 let rd=normalize((eye.world*vec4(mix(eye.fov.x,eye.fov.y,v.uv.x),mix(eye.fov.z,eye.fov.w,v.uv.y),-1.,0.)).xyz);
 let ro=eye.world[3].xyz;
 var color=vec3(0.08,0.15,0.3)+max(rd.y,0.)*0.3;
 var nearest=10000.;
 if rd.y < -0.001 {
  let t=(-1.3-ro.y)/rd.y;
  if t>0. { let p=ro+rd*t; let c=f32((i32(floor(p.x))+i32(floor(p.z))) & 1); color=mix(vec3(0.18),vec3(0.55),c); nearest=t; }
 }
 let oc=ro-vec3(0.,0.,-2.); let b=dot(oc,rd); let c=dot(oc,oc)-0.25;
 let d=b*b-c;
 if d>0. { let t=-b-sqrt(d); if t>0. && t<nearest { let n=normalize(ro+rd*t-vec3(0.,0.,-2.)); color=vec3(0.55,0.15,0.8)*(0.2+0.8*max(dot(n,normalize(vec3(-1.,2.,1.))),0.)); } }
 return vec4(color,1.);
}
