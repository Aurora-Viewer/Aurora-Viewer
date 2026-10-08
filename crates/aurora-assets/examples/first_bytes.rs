use std::collections::HashMap;
fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let mut h: HashMap<String, usize> = HashMap::new();
    let mut sample: HashMap<String, String> = HashMap::new();
    for sub in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(rd) = std::fs::read_dir(sub.path()) else { continue };
        for e in rd.flatten() {
            let Ok(d) = std::fs::read(e.path()) else { continue };
            let k: String = d.iter().take(4).map(|b| format!("{b:02x}")).collect();
            *h.entry(k.clone()).or_default() += 1;
            sample.entry(k).or_insert(e.path().display().to_string());
        }
    }
    let mut v: Vec<_> = h.into_iter().collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (k, n) in v.iter().take(15) {
        println!("{k} {n} {}", sample[k]);
    }
}
