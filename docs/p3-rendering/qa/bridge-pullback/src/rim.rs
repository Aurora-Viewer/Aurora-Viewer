use super::*;

// Parallel-sided bent metal strip. In each y interval x and z vary linearly,
// so the render mesh and the three analytic slab tests describe the same volume.
#[derive(Clone, Copy)]
struct Rib {
    y: f32,
    end: f32,
    x: f32,
    z: f32,
    sx: f32,
    sz: f32,
    w: f32,
    d: f32,
}
impl Rib {
    fn trace(&self, o: Vec3, ray: Vec3) -> Option<f32> {
        let rel = o.y - self.y;
        let pos = [
            rel,
            o.x - self.x - self.sx * rel,
            o.z - self.z - self.sz * rel,
        ];
        let dir = [ray.y, ray.x - self.sx * ray.y, ray.z - self.sz * ray.y];
        let lo = [0., -self.w, -self.d];
        let hi = [self.end - self.y, self.w, self.d];
        let (mut near, mut far) = (0f32, 1e6f32);
        for n in 0..3 {
            if dir[n].abs() < 1e-10 {
                if pos[n] < lo[n] || pos[n] > hi[n] {
                    return None;
                }
            } else {
                let a = (lo[n] - pos[n]) / dir[n];
                let b = (hi[n] - pos[n]) / dir[n];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return None;
                }
            }
        }
        Some(near)
    }
    fn mesh(&self, v: &mut Vec<[f32; 4]>) {
        let p = |i: usize| {
            let y = if i & 2 != 0 { self.end } else { self.y };
            let dy = y - self.y;
            [
                self.x + self.sx * dy + if i & 1 != 0 { self.w } else { -self.w },
                y,
                self.z + self.sz * dy + if i & 4 != 0 { self.d } else { -self.d },
                0.,
            ]
        };
        for ids in [
            [0, 1, 3, 2],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 3, 7, 5],
        ] {
            v.extend([0, 1, 2, 0, 2, 3].map(|i| p(ids[i])));
        }
    }
}
const CURVE_X: f32 = 12.;
const CURVE_Y: f32 = 5.;
fn skin_z(x: f32, y: f32) -> f32 {
    -CURVE_X * x * x - CURVE_Y * y * y
}
fn skin_n(p: Vec3) -> Vec3 {
    Vec3::new(2. * CURVE_X * p.x, 2. * CURVE_Y * p.y, 1.).normalize()
}
fn skin_hit(o: Vec3, d: Vec3) -> Option<f32> {
    let a = CURVE_X * d.x * d.x + CURVE_Y * d.y * d.y;
    let b = d.z + 2. * (CURVE_X * o.x * d.x + CURVE_Y * o.y * d.y);
    let c = o.z + CURVE_X * o.x * o.x + CURVE_Y * o.y * o.y;
    let det = b * b - 4. * a * c;
    if det < 0. || b >= 0. {
        None
    } else {
        Some(2. * c / (-b + det.sqrt()))
    }
}
fn skin_mesh() -> Vec<[f32; 4]> {
    let mut v = Vec::new();
    // 0.75 mm grid, <2.4 micrometres maximum departure from analytic surface.
    for j in -58..58 {
        for i in -75..75 {
            let p = |a: i32, b: i32| {
                let x = (i + a) as f32 * 0.00075;
                let y = (j + b) as f32 * 0.00075;
                [x, y, skin_z(x, y), 0.]
            };
            v.extend([p(0, 0), p(1, 0), p(1, 1), p(0, 0), p(1, 1), p(0, 1)]);
        }
    }
    v
}
fn bend_scene(w: u32, h: u32, eye: Vec3, light: Vec3, ribs: &[Rib]) -> Scene {
    let f = 1. / 30.0_f32.to_radians().tan();
    let proj = Mat4::from_cols(
        Vec4::new(f * h as f32 / w as f32, 0., 0., 0.),
        Vec4::new(0., f, 0., 0.),
        Vec4::new(0., 0., 0., -1.),
        Vec4::new(0., 0., 0.01, 0.),
    );
    let mut depth = Vec::new();
    let mut receiver = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let ray = Vec3::new(
                (2. * (x as f32 + 0.5) / w as f32 - 1.) / proj.x_axis.x,
                (1. - 2. * (y as f32 + 0.5) / h as f32) / f,
                -1.,
            );
            let surface = skin_hit(eye, ray).filter(|t| {
                let p = eye + ray * *t;
                p.x.abs() < 0.056 && p.y.abs() < 0.043
            });
            let near = ribs
                .iter()
                .filter_map(|r| r.trace(eye, ray))
                .min_by(f32::total_cmp);
            let skin = surface.is_some() && near.is_none_or(|n| n > surface.unwrap());
            let t = if skin { surface } else { near };
            depth.push(t.filter(|v| *v > 0.01).map_or(0., |v| 0.01 / v));
            receiver.push(skin);
        }
    }
    Scene {
        name: "curved skin and bent metal frame".into(),
        w,
        h,
        proj,
        depth,
        receiver,
        light,
    }
}
fn disk(light: Vec3, deg: f32) -> Vec<Vec3> {
    let u = Vec3::Y.cross(light).normalize();
    let v = light.cross(u);
    (0..256)
        .map(|i| {
            let a = i as f32 * 2.3999632;
            let r = ((i as f32 + 0.5) / 256.).sqrt() * (0.5 * deg.to_radians()).tan();
            light + r * (a.cos() * u + a.sin() * v)
        })
        .collect()
}

pub(super) fn run() {
    let (map, adapter) = map::Gpu::new();
    let (bend, _) = Gpu::new();
    let w = 640;
    let h = 400;
    let gap = 0.009;
    let width = 0.0018;
    let light = Vec3::new(0.67, 0.14, 0.73).normalize();
    let mut ribs = Vec::new();
    let pos = |y: f32| {
        let x = 0.010 + 0.11 * y + 13. * y * y;
        (x, skin_z(x, y) + gap + 0.035 * y)
    };
    for i in -6..6 {
        let y = i as f32 * 0.004;
        let end = y + 0.004;
        let (x, z) = pos(y);
        let (x1, z1) = pos(end);
        ribs.push(Rib {
            y,
            end,
            x,
            z,
            sx: (x1 - x) / 0.004,
            sz: (z1 - z) / 0.004,
            w: width / 2.,
            d: 0.0005,
        });
    }
    let mut verts = skin_mesh();
    for rib in &ribs {
        rib.mesh(&mut verts);
    }
    let value = |name: &str| std::env::var(name).unwrap().parse::<f32>().unwrap();
    let dirs = disk(light, value("SUN_DIAMETER_DEG"));
    let (nx, ny) = (201, 57);
    let (x0, y0, dx, dy) = (-0.014, -0.014, 0.000175, 0.0005);
    let mut pts = Vec::new();
    let mut norms = Vec::new();
    let mut truth = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            let x = x0 + i as f32 * dx;
            let y = y0 + j as f32 * dy;
            let p = Vec3::new(x, y, skin_z(x, y));
            pts.push(p);
            norms.push(skin_n(p));
            truth.push(
                dirs.iter()
                    .filter(|ray| !ribs.iter().any(|rib| rib.trace(p, **ray).is_some()))
                    .count() as u16,
            );
        }
    }
    let mut frames = Vec::new();
    for distance in [0.16, 0.27, 0.32, 0.36, 0.40, 0.44, 0.49, 0.60, 0.80] {
        let eye = Vec3::new(0., 0., distance);
        let scene = bend_scene(w, h, eye, light, &ribs);
        let (raw, filtered) = bend.run(&scene, 0.05);
        let mut valid = Vec::new();
        let mut raw_vis = Vec::new();
        let mut vis = Vec::new();
        for p in &pts {
            let clip = scene.proj * (*p - eye).extend(1.);
            let x = ((0.5 + 0.5 * clip.x / clip.w) * w as f32).floor() as i32;
            let y = ((0.5 - 0.5 * clip.y / clip.w) * h as f32).floor() as i32;
            assert!(x >= 0 && x < w as i32 && y >= 0 && y < h as i32);
            let k = y as usize * w as usize + x as usize;
            valid.push(scene.receiver[k]);
            raw_vis.push(255 - raw[k][0]);
            vis.push(255 - filtered[k][0]);
        }
        let shadows = map.evaluate_normals(
            eye,
            eye + Vec3::NEG_Z,
            scene.proj,
            (w, h),
            light,
            (value("DETAIL"), value("RANGES")),
            &verts,
            &pts,
            &norms,
            &vis,
        );
        let to_u16 = |v: f32| (v.clamp(0., 1.) * 65535.).round() as u16;
        let mapped: Vec<_> = shadows.iter().map(|s| to_u16(s[0])).collect();
        let combined: Vec<_> = shadows.iter().map(|s| to_u16(s[1])).collect();
        frames.push(
            json!({"eye":eye.to_array(),"map":mapped,"combined":combined,
                           "bend":vis,"bend_raw":raw_vis,"visible":valid}),
        );
    }
    let out = json!({"adapter":adapter,"w":w,"h":h,"nx":nx,"ny":ny,"x0":x0,
                   "y0":y0,"dx":dx,"dy":dy,"gap":gap,"rim_width":width,
                   "truth256":truth,"frames":frames});
    std::fs::write(
        std::env::var("RESULT").unwrap(),
        serde_json::to_vec(&out).unwrap(),
    )
    .unwrap();
}
