//! Diagnostic: components and alpha histogram of a J2C file.
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let data = std::fs::read(&path).unwrap();
    let info = aurora_assets::j2k_info(&data);
    println!("info {info:?}");
    let img = aurora_assets::decode_j2k(&data, 0).unwrap();
    println!("{}x{} components {}", img.width, img.height, img.components);
    if img.components >= 4 {
        let c = img.components as usize;
        let mut hist = [0usize; 5];
        for p in img.data.chunks(c) {
            let a = p[3];
            hist[match a {
                0 => 0,
                1..=63 => 1,
                64..=191 => 2,
                192..=254 => 3,
                _ => 4,
            }] += 1;
        }
        println!("alpha 0 / <64 / mid / >191 / 255: {hist:?}");
    }
}
// (appended) save a PNG next to the scratchpad for inspection
