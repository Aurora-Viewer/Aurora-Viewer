//! Diagnostic: decode a J2C (first 4 channels) to PNG. Usage: j2k_png <in> <out.png>
fn main() {
    let mut a = std::env::args().skip(1);
    let (i, o) = (a.next().expect("in"), a.next().expect("out"));
    let img = aurora_assets::decode_j2k(&std::fs::read(&i).unwrap(), 1).unwrap();
    let c = img.components as usize;
    let mut rgba = Vec::with_capacity((img.width * img.height * 4) as usize);
    for p in img.data.chunks(c) {
        rgba.extend_from_slice(&[p[0], p[1.min(c - 1)], p[2.min(c - 1)], if c >= 4 { p[3] } else { 255 }]);
    }
    image::save_buffer(&o, &rgba, img.width, img.height, image::ColorType::Rgba8).unwrap();
}
