use super::*;
use crate::looks::parse::AuroraSection;

/// LLImageFilter as it is (indra/llimage/llimagefilter.cpp), on an RGBA
/// image whose rows go up like LLImageRaw's: the reference the shader is
/// compared with.
struct Reference {
    w: usize,
    h: usize,
    data: Vec<u8>,
    shape: StencilShape,
    mode: StencilMode,
    min: f32,
    max: f32,
    center: (i32, i32),
    width: i32,
    gamma: f32,
    wavelength: f32,
    sin: f32,
    cos: f32,
    start: (f32, f32),
    grad: (f32, f32),
    grad_n: f32,
    histogram: Option<[u32; 256]>,
}

impl Reference {
    fn new(w: usize, h: usize, data: Vec<u8>) -> Reference {
        Reference {
            w,
            h,
            data,
            shape: StencilShape::Uniform,
            mode: StencilMode::Blend,
            min: 0.0,
            max: 1.0,
            center: (0, 0),
            width: 0,
            gamma: 1.0,
            wavelength: 10.0,
            sin: 0.0,
            cos: 1.0,
            start: (0.0, 0.0),
            grad: (0.0, 0.0),
            grad_n: 0.0,
            histogram: None,
        }
    }

    fn run(&mut self, ops: &[LookOp]) {
        for op in ops {
            match *op {
                LookOp::Stencil {
                    shape,
                    mode,
                    min,
                    max,
                    params,
                } => self.set_stencil(shape, mode, min, max, params),
                LookOp::Gamma { gamma, alpha } => {
                    let lut = |a: f32| {
                        let mut t = [0u8; 256];
                        for (i, v) in t.iter_mut().enumerate() {
                            let g = (i as f32 / 255.0).powf(1.0 / gamma).clamp(0.0, 1.0);
                            *v = ((1.0 - a) * i as f32 + a * 255.0 * g) as u8;
                        }
                        t
                    };
                    self.color_correct(&lut(alpha[0]), &lut(alpha[1]), &lut(alpha[2]));
                }
                LookOp::Colorize { color, alpha } => {
                    let lut = |c: f32, a: f32| {
                        let mut t = [0u8; 256];
                        for (i, v) in t.iter_mut().enumerate() {
                            *v = (((1.0 - a) * i as f32 + 255.0 * a * c) as i32).clamp(0, 255) as u8;
                        }
                        t
                    };
                    self.color_correct(&lut(color[0], alpha[0]), &lut(color[1], alpha[1]), &lut(color[2], alpha[2]));
                }
                LookOp::Contrast { slope, alpha } => {
                    let translate = 128.0 * (1.0 - slope);
                    self.blend_luts(alpha, |i| ((slope * i as f32 + translate) as i32).clamp(0, 255) as u8);
                }
                LookOp::Brightness { step, alpha } => {
                    let add = (step * 255.0) as i32;
                    self.blend_luts(alpha, |i| (i as i32 + add).clamp(0, 255) as u8);
                }
                LookOp::Linearize { tail, alpha } => self.linearize(tail, alpha),
                LookOp::Posterize { classes, alpha } => self.equalize(classes as i32, alpha),
                LookOp::Screen { mode, wavelength, angle } => self.screen(mode, wavelength, angle),
                LookOp::Convolve { kernel, normalize, abs } => self.convolve(&kernel, normalize, abs),
                _ => self.color_transform(&color_rows(op).expect("a colour operation")),
            }
        }
    }

    fn px(&self, i: usize, j: usize) -> usize {
        (j * self.w + i) * 4
    }

    fn set_stencil(&mut self, shape: StencilShape, mode: StencilMode, min: f32, max: f32, p: [f32; 4]) {
        let (w, h) = (self.w as f32, self.h as f32);
        self.shape = shape;
        self.mode = mode;
        self.min = min.clamp(-1.0, 1.0);
        self.max = max.clamp(-1.0, 1.0);
        self.center = ((w + p[0] * h) as i32 / 2, (h + p[1] * h) as i32 / 2);
        self.width = (p[2] * h) as i32 / 2;
        self.gamma = if p[3] <= 0.0 { 1.0 } else { p[3] };
        self.wavelength = if p[0] <= 0.0 { 10.0 } else { p[0] * h / 2.0 };
        self.sin = p[1].to_radians().sin();
        self.cos = p[1].to_radians().cos();
        self.start = ((w + p[0] * h) / 2.0, (h + p[1] * h) / 2.0);
        let end = ((w + p[2] * h) / 2.0, (h + p[3] * h) / 2.0);
        self.grad = (end.0 - self.start.0, end.1 - self.start.1);
        self.grad_n = self.grad.0 * self.grad.0 + self.grad.1 * self.grad.1;
    }

    fn alpha(&self, i: usize, j: usize) -> f32 {
        let (i32_, j32) = (i as i32, j as i32);
        let alpha = match self.shape {
            StencilShape::Uniform => 1.0,
            StencilShape::Vignette => {
                let d = ((i32_ - self.center.0) * (i32_ - self.center.0) + (j32 - self.center.1) * (j32 - self.center.1)) as f32;
                std::f32::consts::E.powf(-((d / (self.width * self.width) as f32).powf(self.gamma) / 2.0))
            }
            StencilShape::ScanLines => {
                let d = self.sin * i as f32 - self.cos * j as f32;
                if (2.0 * std::f32::consts::PI * d / self.wavelength).sin() > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            StencilShape::Gradient => {
                (((i as f32 - self.start.0) * self.grad.0 + (j as f32 - self.start.1) * self.grad.1) / self.grad_n).clamp(0.0, 1.0)
            }
        };
        self.min + alpha * (self.max - self.min)
    }

    fn blend(&mut self, at: usize, alpha: f32, c: [u8; 3]) {
        let inv = 1.0 - alpha;
        for k in 0..3 {
            let p = self.data[at + k] as f32;
            let c = c[k] as f32;
            self.data[at + k] = match self.mode {
                StencilMode::Blend => (inv * p + alpha * c) as u8,
                StencilMode::Add => (p + alpha * c).clamp(0.0, 255.0) as u8,
                StencilMode::AddBack => (inv * p + c).clamp(0.0, 255.0) as u8,
                StencilMode::Fade => (alpha * c) as u8,
            };
        }
    }

    fn color_correct(&mut self, r: &[u8; 256], g: &[u8; 256], b: &[u8; 256]) {
        for j in 0..self.h {
            for i in 0..self.w {
                let at = self.px(i, j);
                let c = [
                    r[self.data[at] as usize],
                    g[self.data[at + 1] as usize],
                    b[self.data[at + 2] as usize],
                ];
                self.blend(at, self.alpha(i, j), c);
            }
        }
    }

    /// A curve of values blended with the level by the channel's alpha.
    fn blend_luts(&mut self, alpha: [f32; 3], value: impl Fn(usize) -> u8) {
        let lut = |a: f32| {
            let mut t = [0u8; 256];
            for (i, v) in t.iter_mut().enumerate() {
                *v = ((1.0 - a) * i as f32 + a * value(i) as f32) as u8;
            }
            t
        };
        self.color_correct(&lut(alpha[0]), &lut(alpha[1]), &lut(alpha[2]));
    }

    fn color_transform(&mut self, rows: &M3) {
        for j in 0..self.h {
            for i in 0..self.w {
                let at = self.px(i, j);
                let s = [self.data[at] as f32, self.data[at + 1] as f32, self.data[at + 2] as f32];
                let c = rows.map(|r| (s[0] * r[0] + s[1] * r[1] + s[2] * r[2]).clamp(0.0, 255.0) as u8);
                self.blend(at, self.alpha(i, j), c);
            }
        }
    }

    fn convolve(&mut self, kernel: &M3, normalize: bool, abs: bool) {
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for &v in kernel.iter().flatten() {
            if v >= 0.0 {
                hi += v;
            } else {
                lo += v;
            }
        }
        if abs {
            hi = hi.abs().max(lo.abs());
            lo = 0.0;
        }
        let range = hi - lo;
        let src = self.data.clone();
        for j in 0..self.h {
            for i in 0..self.w {
                let at = self.px(i, j);
                if j == 0 || j == self.h - 1 || i == 0 || i == self.w - 1 {
                    // the last line takes the first one's stencil
                    let aj = if j == self.h - 1 { 0 } else { j };
                    self.blend(at, self.alpha(i, aj), [0; 3]);
                    continue;
                }
                let mut c = [0u8; 3];
                for (k, out) in c.iter_mut().enumerate() {
                    let mut v = 0.0f32;
                    for (r, row) in kernel.iter().enumerate() {
                        for (col, weight) in row.iter().enumerate() {
                            v += weight * src[self.px(i + col - 1, j + r - 1) + k] as f32;
                        }
                    }
                    if abs {
                        v = v.abs();
                    }
                    if normalize {
                        v = (v - lo) / range;
                    }
                    *out = v.clamp(0.0, 255.0) as u8;
                }
                self.blend(at, self.alpha(i, j), c);
            }
        }
    }

    fn screen(&mut self, mode: ScreenMode, wavelength: f32, angle: f32) {
        let wl = wavelength * self.h as f32 / 2.0;
        let (sin, cos) = angle.to_radians().sin_cos();
        let mut gamma = [0u8; 256];
        for (i, g) in gamma.iter_mut().enumerate() {
            *g = (255.0 * (i as f32 / 255.0).powf(0.25).clamp(0.0, 1.0)) as u8;
        }
        let tau = 2.0 * std::f32::consts::PI;
        for j in 0..self.h {
            for i in 0..self.w {
                let (fi, fj) = (i as f32, j as f32);
                let value = match mode {
                    ScreenMode::Sine2d => {
                        let di = cos * fi + sin * fj;
                        let dj = -sin * fi + cos * fj;
                        ((tau * di / wl).sin() * (tau * dj / wl).sin() + 1.0) * 255.0 / 2.0
                    }
                    ScreenMode::Line => ((tau * (sin * fi - cos * fj) / wl).sin() + 1.0) * 255.0 / 2.0,
                } as u8;
                let at = self.px(i, j);
                let red = self.data[at];
                let v = if red >= value { gamma[(red - value) as usize] } else { 0 };
                self.blend(at, self.alpha(i, j), [v; 3]);
            }
        }
    }

    fn cumulated(&mut self) -> [u32; 256] {
        if self.histogram.is_none() {
            let mut bins = [0u32; 256];
            for p in self.data.as_chunks::<4>().0 {
                bins[(p[0] as usize + p[1] as usize + p[2] as usize) / 3] += 1;
            }
            self.histogram = Some(bins);
        }
        let bins = self.histogram.expect("counted");
        let mut sum = 0;
        bins.map(|n| {
            sum += n;
            sum
        })
    }

    fn linearize(&mut self, tail: f32, alpha: [f32; 3]) {
        let cumulated = self.cumulated();
        let tail = tail.clamp(0.0, 1.0);
        let total = cumulated[255];
        let min_c = (total as f32 * tail) as u32;
        let max_c = (total as f32 * (1.0 - tail)) as u32;
        let mut min_v = 0usize;
        while min_v < 255 && cumulated[min_v] < min_c {
            min_v += 1;
        }
        let mut max_v = 255usize;
        while max_v > 0 && cumulated[max_v] > max_c {
            max_v -= 1;
        }
        if max_v == min_v {
            self.blend_luts(alpha, |i| if i < min_v { 0 } else { 255 });
        } else {
            let slope = 255.0 / (max_v as f32 - min_v as f32);
            let translate = -(min_v as f32) * slope;
            self.blend_luts(alpha, |i| ((slope * i as f32 + translate) as i32).clamp(0, 255) as u8);
        }
    }

    fn equalize(&mut self, classes: i32, alpha: [f32; 3]) {
        let classes = classes.clamp(2, 255) as u32;
        let cumulated = self.cumulated();
        let total = cumulated[255];
        let delta_count = total / classes;
        let delta_value = 256 / (classes - 1);
        let mut values = [0u8; 256];
        let (mut count, mut value) = (delta_count, 0u32);
        for (i, v) in values.iter_mut().enumerate() {
            *v = value as u8;
            if cumulated[i] >= count {
                count += delta_count;
                value = (value + delta_value).min(255);
            }
        }
        self.blend_luts(alpha, |i| values[i]);
    }
}

#[test]
fn every_legacy_look_reads() {
    let looks = legacy_looks();
    assert_eq!(looks.len(), 30);
    for (name, ops) in looks {
        assert!(!ops.is_empty(), "{name}");
        assert!(plan(ops).passes.len() < MAX_PASSES, "{name}");
    }
    let miniature = legacy_look("Miniature").expect("Miniature");
    assert_eq!(miniature.len(), 27);
    assert_eq!(miniature.iter().filter(|op| matches!(op, LookOp::Convolve { .. })).count(), 21);
    assert_eq!(
        legacy_look("Negative").as_deref(),
        Some(
            &[LookOp::Linearize {
                tail: 0.9,
                alpha: [1.0; 3]
            }][..]
        )
    );
}

#[test]
fn a_filter_file_reads_like_the_legacy_viewer() {
    let xml = br#"<llsd><array>
        <array><string>stencil</string><string>vignette</string><string>fade</string><real>0.5</real><real>1.0</real><real>0.1</real></array>
        <array><string>darken</string><real>0.2</real><real>1.0</real><real>0.5</real><real>0.0</real></array>
        <array><string>posterize</string><real>4.7</real></array>
        <array><string>sparkle</string></array>
        <array><string>screen</string><string>dots</string><real>0.02</real></array>
    </array></llsd>"#;
    let ops = parse_look(xml).expect("reads");
    assert_eq!(
        ops,
        [
            LookOp::Stencil {
                shape: StencilShape::Vignette,
                mode: StencilMode::Fade,
                min: 0.5,
                max: 1.0,
                params: [0.1, 0.0, 0.0, 0.0]
            },
            LookOp::Brightness {
                step: -0.2,
                alpha: [1.0, 0.5, 0.0]
            },
            LookOp::Posterize {
                classes: 4.0,
                alpha: [0.0; 3]
            },
            // the unknown operation is left out, the unknown screen is the first
            LookOp::Screen {
                mode: ScreenMode::Sine2d,
                wavelength: 0.02,
                angle: 0.0
            },
        ]
    );
    assert!(parse_look(b"<llsd><map></map></llsd>").is_err());
}

#[test]
fn passes_split_at_convolutions_and_before_the_first_histogram_curve() {
    let curve = LookOp::Linearize {
        tail: 0.1,
        alpha: [1.0; 3],
    };
    let stencil = LookOp::Stencil {
        shape: StencilShape::Vignette,
        mode: StencilMode::Blend,
        min: 0.0,
        max: 1.0,
        params: [0.0, 0.0, 1.0, 2.0],
    };
    // Posterize: blur darken contrast linearize posterize
    let p = plan(&[
        LookOp::blur(),
        LookOp::Brightness {
            step: -0.1,
            alpha: [1.0; 3],
        },
        curve,
        LookOp::Posterize {
            classes: 5.0,
            alpha: [1.0; 3],
        },
    ]);
    assert_eq!(p.passes, [(0, 2), (2, 4)]);
    assert_eq!(p.histogram_pass, Some(1));
    assert_eq!(p.curves, [[2, 0], [3, 1]]);
    // a stencil alone changes nothing: no pass for it
    let p = plan(&[stencil, curve, LookOp::GrayScale, stencil, LookOp::blur(), LookOp::blur()]);
    assert_eq!(p.passes, [(0, 4), (4, 5), (5, 6)]);
    assert_eq!(p.histogram_pass, Some(0));
}

#[test]
fn colour_rows_follow_the_legacy_matrices() {
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for op in [LookOp::Saturate(1.0), LookOp::Rotate(0.0), LookOp::Rotate(360.0)] {
        let rows = color_rows(&op).expect("colour");
        for (r, i) in rows.iter().zip(identity) {
            for (a, b) in r.iter().zip(i) {
                assert!((a - b).abs() < 1e-5, "{op:?}: {rows:?}");
            }
        }
    }
    // no saturation: every channel the mean of the three
    let grey = color_rows(&LookOp::Saturate(0.0)).expect("colour");
    for row in grey {
        for v in row {
            assert!((v - 1.0 / 3.0).abs() < 1e-5, "{grey:?}");
        }
    }
    // a half turn of the hues keeps the greys
    let half = color_rows(&LookOp::Rotate(180.0)).expect("colour");
    for row in half {
        assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{half:?}");
    }
}

/// A test photo: soft gradients with a few hard edges and some noise.
fn test_image(w: usize, h: usize) -> Vec<u8> {
    let mut seed = 0x2545_f491u32;
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed % 23) as i32 - 11;
            let edge = if (x / 9 + y / 7) % 3 == 0 { 70 } else { 0 };
            let r = (x * 255 / w) as i32 + noise;
            let g = (y * 255 / h) as i32 + edge - noise / 2;
            let b = 255 - ((x + y) * 255 / (w + h)) as i32 + noise;
            out.extend([r, g, b].map(|v| v.clamp(0, 255) as u8));
            out.push(255);
        }
    }
    out
}

fn flip_rows(data: &[u8], w: usize) -> Vec<u8> {
    data.chunks_exact(w * 4).rev().flatten().copied().collect()
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// The adapter is the virtual GPU of the macOS runners (macos.yml, "Apple
/// Paravirtual device"), whose Metal arithmetic rounds further than a real
/// Apple GPU's.
fn virtual_apple_gpu() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&Default::default())).is_ok_and(|a| a.get_info().name.contains("Paravirtual"))
}

/// The look drawn by the GPU over `pixels` (rows going down) in `format`.
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    look: &PhotoLook,
    size: (u32, u32),
    pixels: &[u8],
) -> Vec<u8> {
    let mut looks = Looks::new(device);
    looks.prepare(device, format, size);
    let input = looks.targets.as_ref().map(|t| t.input.texture().clone()).expect("prepared");
    let extent = wgpu::Extent3d {
        width: size.0,
        height: size.1,
        depth_or_array_layers: 1,
    };
    let row = size.0 * 4;
    let layout = wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(row),
        rows_per_image: Some(size.1),
    };
    queue.write_texture(input.as_image_copy(), pixels, layout, extent);
    // a read back's rows are aligned
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("photo look test"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("photo look test"),
        size: (padded * size.1) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    looks.encode(device, queue, &mut encoder, look, &target.create_view(&Default::default()), None);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                bytes_per_row: Some(padded),
                ..layout
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    read.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let mapped = read.get_mapped_range(..).expect("mapped");
    mapped
        .chunks_exact(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}

fn look_over(ops: Arc<[LookOp]>, size: (u32, u32)) -> PhotoLook {
    PhotoLook {
        layers: vec![LookLayer {
            ops,
            intensity: 1.0,
            lut: None,
        }],
        compare: None,
        image: [0.0, 0.0, size.0 as f32, size.1 as f32],
        scale: 1.0,
        hold: false,
        focus: None,
    }
}

/// The mean difference of the levels, and how many are further apart than
/// the GPU's arithmetic explains: its pow, exp and sin are a little off, so
/// a truncation falls the other side of a level now and then, carried by
/// the next operations, most where posterize's classes and a screen's
/// edges amplify them (on 2026-10-26: at most 0.07 level on average and 33
/// of 18,432 levels by more than 2 on an RTX 4090; 0.18 and 225, The
/// Matrix, on the CI runner's software adapter). A wrong operation is
/// levels off everywhere.
fn differences(a: &[u8], b: &[u8]) -> (f64, usize) {
    let mut sum = 0u64;
    let mut far = 0;
    for (x, y) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        for k in 0..3 {
            let d = (x[k] as i32 - y[k] as i32).unsigned_abs();
            sum += d as u64;
            far += usize::from(d > 2);
        }
    }
    (sum as f64 / (a.len() / 4 * 3) as f64, far)
}

#[test]
fn the_gpu_draws_every_legacy_look_like_the_legacy_viewer() {
    let _gpu = crate::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (w, h) = (96usize, 64usize);
    let image = test_image(w, h);
    // the bounds of `differences`: a mean under 0.35 level and fewer than 1
    // level in 50 further than 2; on the macOS runners' virtual GPU, 0.6 and
    // 1 in 16 (The Matrix measured 0.42 and 738 of 18,432 there on
    // 2026-10-29, every other look within the usual bounds; a real Apple
    // GPU passes the usual ones)
    let (mean_max, far_share) = if virtual_apple_gpu() { (0.6, 16) } else { (0.35, 50) };
    let mut failed = Vec::new();
    for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
        for (name, ops) in legacy_looks() {
            let mut reference = Reference::new(w, h, flip_rows(&image, w));
            reference.run(ops);
            let expected = flip_rows(&reference.data, w);
            let got = draw(
                &device,
                &queue,
                format,
                &look_over(ops.clone(), (w as u32, h as u32)),
                (w as u32, h as u32),
                &image,
            );
            let (mean, far) = differences(&expected, &got);
            if mean >= mean_max || far * far_share >= w * h * 3 {
                failed.push(format!("{name} ({format:?}): mean {mean:.3}, {far} levels further than 2"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn intensity_and_comparison_keep_the_original() {
    let _gpu = crate::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (w, h) = (64u32, 32u32);
    let image = test_image(w as usize, h as usize);
    let negative = legacy_look("Negative").expect("Negative");
    let mut look = look_over(negative, (w, h));
    let full = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, (w, h), &image);
    look.layers[0].intensity = 0.0;
    let none = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, (w, h), &image);
    assert_eq!(none, image);
    look.layers[0].intensity = 0.5;
    let half = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, (w, h), &image);
    for ((a, b), c) in image.iter().zip(&full).zip(&half) {
        assert!((*a as f32 * 0.5 + *b as f32 * 0.5 - *c as f32).abs() <= 1.0);
    }
    // the left quarter as it is, the rest negative
    look.layers[0].intensity = 1.0;
    look.compare = Some(0.25);
    let compared = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, (w, h), &image);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = (y * w as usize + x) * 4;
            let expected = if x < w as usize / 4 { &image } else { &full };
            assert_eq!(compared[at..at + 4], expected[at..at + 4], "{x}, {y}");
        }
    }
}

#[test]
fn the_photo_inside_a_larger_view_keeps_its_own_coordinates() {
    let _gpu = crate::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    // a 48 x 32 photo drawn at the middle of an 80 x 48 view: the view's
    // pixels around it untouched, the vignette centred on the photo
    let (w, h) = (80u32, 48u32);
    let view = test_image(w as usize, h as usize);
    let ops: Arc<[LookOp]> = legacy_look("Toycamera").expect("Toycamera");
    let mut look = look_over(ops.clone(), (48, 32));
    look.image = [16.0, 8.0, 48.0, 32.0];
    let got = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, (w, h), &view);
    let photo: Vec<u8> = (8..40usize)
        .flat_map(|y| view[(y * 80 + 16) * 4..(y * 80 + 64) * 4].to_vec())
        .collect();
    let alone = draw(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        &look_over(ops, (48, 32)),
        (48, 32),
        &photo,
    );
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = (y * 80 + x) * 4;
            if (16..64).contains(&x) && (8..40).contains(&y) {
                let inner = ((y - 8) * 48 + x - 16) * 4;
                assert_eq!(got[at..at + 4], alone[inner..inner + 4], "{x}, {y}");
            } else {
                assert_eq!(got[at..at + 4], view[at..at + 4], "{x}, {y}");
            }
        }
    }
}

#[test]
fn looks_shader_is_valid() {
    let module = naga::front::wgsl::parse_str(include_str!("../shaders/looks.wgsl")).expect("parses");
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .expect("valid");
}

#[test]
fn every_aurora_look_reads_into_its_section() {
    let looks = aurora_looks();
    assert_eq!(looks.len(), 36);
    for look in looks {
        assert!(!look.ops.is_empty(), "{}", look.id);
        assert!(plan(&look.ops).passes.len() < MAX_PASSES, "{}", look.id);
    }
    let instant = aurora_look("instant").expect("instant");
    assert_eq!(instant.section, AuroraSection::Film);
    assert!(matches!(instant.ops.last(), Some(LookOp::Frame { .. })));
    assert!(plan(&aurora_look("subject").expect("subject").ops).focus);
}

#[test]
fn operations_that_read_around_start_a_pass() {
    let p = plan(&[LookOp::Saturate(1.2), LookOp::Chroma(0.01), LookOp::GrayScale, LookOp::Distort(0.2)]);
    assert_eq!(p.passes, [(0, 1), (1, 3), (3, 4)]);
    assert!(!p.focus);
}

fn draw_ops(ops: Vec<LookOp>, size: (u32, u32), image: &[u8], lut: Option<Arc<Lut>>) -> Option<Vec<u8>> {
    let _gpu = crate::test_gpu::lock();
    let (device, queue) = gpu()?;
    let mut look = look_over(ops.into(), size);
    look.layers[0].lut = lut;
    Some(draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &look, size, image))
}

#[test]
fn selective_colour_keeps_one_hue() {
    // a red pixel, a blue one, a grey one
    let image = [230u8, 20, 30, 255, 30, 40, 220, 255, 120, 120, 120, 255, 200, 30, 20, 255];
    let Some(out) = draw_ops(
        vec![LookOp::Selective {
            hue: 0.0,
            width: 22.0,
            soft: 14.0,
        }],
        (2, 2),
        &image,
        None,
    ) else {
        return;
    };
    assert_eq!(out[0..3], image[0..3]);
    // the blue one in grey
    assert!(out[4].abs_diff(out[6]) <= 1 && out[5].abs_diff(out[6]) <= 1, "{:?}", &out[4..8]);
    assert_eq!(out[8..11], image[8..11]);
}

#[test]
fn bars_leave_the_frame_of_their_ratio() {
    let (w, h) = (64u32, 36u32);
    let image = test_image(w as usize, h as usize);
    let Some(out) = draw_ops(
        vec![LookOp::Bars {
            ratio: 2.39,
            color: [0.0; 3],
        }],
        (w, h),
        &image,
        None,
    ) else {
        return;
    };
    // 64 / 2.39 = 26.8 rows kept: about 4.6 rows of bar at the top and bottom
    let row = |y: usize| &out[y * w as usize * 4..(y + 1) * w as usize * 4];
    assert!(row(0).chunks(4).all(|p| p[..3] == [0, 0, 0]));
    assert!(row(35).chunks(4).all(|p| p[..3] == [0, 0, 0]));
    assert_eq!(row(18), &image[18 * w as usize * 4..19 * w as usize * 4]);
}

#[test]
fn an_identity_table_changes_nothing() {
    let (w, h) = (32u32, 16u32);
    let image = test_image(w as usize, h as usize);
    let Some(out) = draw_ops(vec![LookOp::Lut], (w, h), &image, Some(Arc::new(Lut::identity(33)))) else {
        return;
    };
    let (mean, far) = differences(&image, &out);
    assert!(mean < 0.6 && far == 0, "mean {mean}, {far} far");
}

#[test]
fn every_aurora_look_draws() {
    let _gpu = crate::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (w, h) = (96u32, 64u32);
    let image = test_image(w as usize, h as usize);
    for look in aurora_looks() {
        eprintln!("drawing {}", look.id);
        let out = draw(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &look_over(look.ops.clone(), (w, h)),
            (w, h),
            &image,
        );
        // without the scene's depth, the depth looks leave it as it is
        if !matches!(look.id, "subject" | "background-blur" | "miniature-3d") {
            assert_ne!(out, image, "{} changes nothing", look.id);
        }
    }
}

#[test]
fn stacked_layers_draw_one_after_the_other() {
    let _gpu = crate::test_gpu::lock();
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (w, h) = (64u32, 48u32);
    let image = test_image(w as usize, h as usize);
    let sepia = legacy_look("Sepia").expect("Sepia");
    let toy = legacy_look("Toycamera").expect("Toycamera");
    let both: Arc<[LookOp]> = sepia.iter().chain(toy.iter()).copied().collect();
    let one = draw(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        &look_over(both, (w, h)),
        (w, h),
        &image,
    );
    let mut stacked = look_over(sepia.clone(), (w, h));
    stacked.layers.push(LookLayer {
        ops: toy.clone(),
        intensity: 1.0,
        lut: None,
    });
    let two = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &stacked, (w, h), &image);
    assert_eq!(one, two);
    // a layer at 0 % leaves the one below as it is
    stacked.layers[1].intensity = 0.0;
    let first = draw(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        &look_over(sepia, (w, h)),
        (w, h),
        &image,
    );
    let under = draw(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, &stacked, (w, h), &image);
    assert_eq!(first, under);
}
