//! Diagnostic: alpha channel of a J2C as a grey PNG (white = opaque).
fn main() {
    let mut a = std::env::args().skip(1);
    let (i, o) = (a.next().expect("in"), a.next().expect("out"));
    let img = aurora_assets::decode_j2k(&std::fs::read(&i).unwrap(), 1).unwrap();
    let c = img.components as usize;
    let mut out = Vec::with_capacity((img.width * img.height * 3) as usize);
    for p in img.data.chunks(c) {
        let v = if c >= 4 { p[3] } else { 255 };
        out.extend_from_slice(&[v, p[0] / 2 + v / 2, v]);
    }
    image::save_buffer(&o, &out, img.width, img.height, image::ColorType::Rgb8).unwrap();
}
