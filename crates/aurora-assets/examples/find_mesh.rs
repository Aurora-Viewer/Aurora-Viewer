fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let mut n = 0;
    for sub in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(rd) = std::fs::read_dir(sub.path()) else { continue };
        for e in rd.flatten() {
            let Ok(d) = std::fs::read(e.path()) else { continue };
            if d.len() < 64 || d[0..4] != [1, 0, 0, 0] {
                continue;
            }
            // find the first binary-LLSD map marker in the first 64 bytes
            let hex: String = d.iter().take(48).map(|b| format!("{b:02x}")).collect();
            if n < 8 {
                println!("{} len {} {hex}", e.path().display(), d.len());
            }
            n += 1;
        }
    }
}
