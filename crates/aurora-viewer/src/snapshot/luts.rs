//! The user's 3D look-up tables for the photo's look (issue #969): .cube
//! files copied into `photo-luts` in the configuration folder, listed by
//! file name and read on a worker when one is shown (a 65³ table is a few
//! megabytes of text). An import reads the chosen file first: only a table
//! that reads is copied.

use aurora_render::looks::{Lut, parse_cube};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The library of tables.
pub struct LutLibrary {
    dir: PathBuf,
    names: Vec<String>,
    listed: bool,
    tables: HashMap<String, Arc<Lut>>,
    /// The tables that did not read, and why.
    broken: HashMap<String, String>,
    /// Those not said yet to the user.
    failures: Vec<String>,
    /// The table being read.
    loading: Option<(String, crossbeam_channel::Receiver<Result<Lut, String>>)>,
}

/// An import's outcome: the file name in the library and its table, or
/// what went wrong.
pub type Imported = Result<(String, Arc<Lut>), String>;

impl LutLibrary {
    pub fn new(dir: PathBuf) -> LutLibrary {
        LutLibrary {
            dir,
            names: Vec::new(),
            listed: false,
            tables: HashMap::new(),
            broken: HashMap::new(),
            failures: Vec::new(),
            loading: None,
        }
    }

    /// The folder of the tables.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The tables, by file name (the folder is read the first time).
    pub fn names(&mut self) -> &[String] {
        if !self.listed {
            self.refresh();
        }
        &self.names
    }

    pub fn refresh(&mut self) {
        self.listed = true;
        self.names = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("cube")))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        self.names.sort_by_cached_key(|n| super::looks::fold(n));
    }

    /// The tables that did not read since the last call: their name and
    /// why.
    pub fn take_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.failures)
    }

    /// A table, once read; asks for it otherwise (one at a time).
    pub fn table(&mut self, name: &str) -> Option<Arc<Lut>> {
        self.poll();
        if let Some(lut) = self.tables.get(name) {
            return Some(lut.clone());
        }
        if self.loading.is_none() && !self.broken.contains_key(name) {
            let path = self.dir.join(name);
            let (tx, rx) = crossbeam_channel::bounded(1);
            std::thread::spawn(move || {
                let read = std::fs::read_to_string(&path).map_err(|e| e.to_string());
                let _ = tx.send(read.and_then(|text| parse_cube(&text).map_err(|e| e.to_string())));
            });
            self.loading = Some((name.to_owned(), rx));
        }
        None
    }

    /// Takes the table read on the worker.
    pub fn poll(&mut self) {
        let Some((name, rx)) = &self.loading else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            // the worker is gone without an answer
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err("not read".to_owned()),
        };
        let name = name.clone();
        self.loading = None;
        match result {
            Ok(lut) => {
                self.tables.insert(name, Arc::new(lut));
            }
            Err(e) => {
                log::warn!("photo LUT {name}: {e}");
                self.failures.push(format!("{}: {e}", super::looks::lut_display(&name)));
                self.broken.insert(name, e);
            }
        }
    }

    /// Reads `path` and copies it into the library, on the calling thread
    /// (a worker): the name it takes and its table.
    pub fn import(dir: &Path, path: &Path) -> Imported {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let lut = parse_cube(&text).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("LUT");
        let mut name = format!("{stem}.cube");
        let mut n = 2;
        while dir.join(&name).exists() {
            name = format!("{stem} ({n}).cube");
            n += 1;
        }
        std::fs::write(dir.join(&name), text).map_err(|e| e.to_string())?;
        Ok((name, Arc::new(lut)))
    }

    /// An import done: its table kept, the list read again.
    pub fn imported(&mut self, name: String, lut: Arc<Lut>) {
        self.broken.remove(&name);
        self.tables.insert(name, lut);
        self.refresh();
    }

    /// Removes a table from the library (the file to the bin is not
    /// possible everywhere: it is deleted).
    pub fn remove(&mut self, name: &str) -> std::io::Result<()> {
        // only a plain file name of the folder
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) {
            return Err(std::io::Error::other("not a file of the library"));
        }
        std::fs::remove_file(self.dir.join(name))?;
        self.tables.remove(name);
        self.broken.remove(name);
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(n: u32) -> String {
        let lut = Lut::identity(n);
        let mut text = format!("LUT_3D_SIZE {n}\n");
        for c in &lut.data {
            text.push_str(&format!("{} {} {}\n", c[0], c[1], c[2]));
        }
        text
    }

    #[test]
    fn imports_list_read_and_remove_tables() {
        let root = std::env::temp_dir().join(format!("aurora-luts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("photo-luts");
        std::fs::create_dir_all(&root).expect("temp");
        let source = root.join("Warm.cube");
        std::fs::write(&source, cube(4)).expect("write");
        let bad = root.join("Bad.cube");
        std::fs::write(&bad, "LUT_3D_SIZE 4\n0 0 0\n").expect("write");

        let mut library = LutLibrary::new(dir.clone());
        assert!(library.names().is_empty());
        assert!(LutLibrary::import(&dir, &bad).is_err());
        let (name, lut) = LutLibrary::import(&dir, &source).expect("imports");
        assert_eq!(name, "Warm.cube");
        library.imported(name, lut);
        // the same file again takes another name
        let (second, lut) = LutLibrary::import(&dir, &source).expect("imports");
        assert_eq!(second, "Warm (2).cube");
        library.imported(second, lut);
        assert_eq!(library.names(), ["Warm (2).cube", "Warm.cube"]);
        assert_eq!(library.table("Warm.cube").map(|l| l.size), Some(4));

        // a table read on the worker
        let mut fresh = LutLibrary::new(dir.clone());
        let mut lut = None;
        for _ in 0..200 {
            lut = fresh.table("Warm.cube");
            if lut.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(lut.map(|l| l.size), Some(4));

        assert!(library.remove("../Warm.cube").is_err());
        library.remove("Warm.cube").expect("removed");
        assert_eq!(library.names(), ["Warm (2).cube"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
