//! 3D look-up tables from .cube files (Adobe / Resolve "Cube LUT
//! Specification" 1.0): `LUT_3D_SIZE n`, an optional domain, then n³ lines
//! of red, green and blue, red changing fastest. The table maps the
//! photo's gamma-encoded colours, as the creative tables of photo and video
//! software do.

/// A 3D table: `size`³ colours, red fastest, then green, then blue.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut {
    pub size: u32,
    pub data: Vec<[f32; 3]>,
}

/// The largest side read (the files go to 65).
pub const MAX_SIZE: u32 = 128;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CubeError {
    #[error("no LUT_3D_SIZE (a 1D table is not supported)")]
    NoSize,
    #[error("size {0} out of 2..={MAX_SIZE}")]
    Size(u32),
    #[error("line {0}: not a colour")]
    Line(usize),
    #[error("{found} colours for {expected}")]
    Count { found: usize, expected: usize },
}

impl Lut {
    /// The table as an RGBA8 3D texture, mapped from its domain to 0..1.
    pub fn rgba8(&self) -> Vec<u8> {
        self.data
            .iter()
            .flat_map(|c| {
                let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                [r, g, b, 255]
            })
            .collect()
    }

    /// The table that changes nothing.
    pub fn identity(size: u32) -> Lut {
        let n = size.max(2);
        let step = 1.0 / (n - 1) as f32;
        let mut data = Vec::with_capacity((n * n * n) as usize);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    data.push([r as f32 * step, g as f32 * step, b as f32 * step]);
                }
            }
        }
        Lut { size: n, data }
    }
}

/// Reads a .cube file. Values outside the domain are brought into 0..1.
pub fn parse_cube(text: &str) -> Result<Lut, CubeError> {
    let mut size = None;
    let mut min = [0.0f32; 3];
    let mut max = [1.0f32; 3];
    let mut data = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else { continue };
        let three = |words: std::str::SplitWhitespace| -> Option<[f32; 3]> {
            let v: Vec<f32> = words.take(3).map(str::parse).collect::<Result<_, _>>().ok()?;
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        };
        match first {
            "TITLE" | "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {}
            "LUT_1D_SIZE" => return Err(CubeError::NoSize),
            "LUT_3D_SIZE" => {
                let n_size: u32 = words.next().and_then(|w| w.parse().ok()).ok_or(CubeError::Line(n + 1))?;
                if !(2..=MAX_SIZE).contains(&n_size) {
                    return Err(CubeError::Size(n_size));
                }
                size = Some(n_size);
                data.reserve((n_size * n_size * n_size) as usize);
            }
            "DOMAIN_MIN" => min = three(words).ok_or(CubeError::Line(n + 1))?,
            "DOMAIN_MAX" => max = three(words).ok_or(CubeError::Line(n + 1))?,
            _ => {
                let mut all = vec![first];
                all.extend(words);
                let v: Vec<f32> = all
                    .iter()
                    .map(|w| w.parse())
                    .collect::<Result<_, _>>()
                    .map_err(|_| CubeError::Line(n + 1))?;
                if v.len() != 3 {
                    return Err(CubeError::Line(n + 1));
                }
                data.push([0, 1, 2].map(|k| (v[k] - min[k]) / (max[k] - min[k]).max(1e-6)));
            }
        }
    }
    let size = size.ok_or(CubeError::NoSize)?;
    let expected = (size * size * size) as usize;
    if data.len() != expected {
        return Err(CubeError::Count {
            found: data.len(),
            expected,
        });
    }
    Ok(Lut { size, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_file_reads_with_its_domain() {
        let text = "# a comment\nTITLE \"Warm\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n\
                    0 0 0\n2 0 0\n0 2 0\n2 2 0\n0 0 2\n2 0 2\n0 2 2\n2 2 2\n";
        let lut = parse_cube(text).expect("reads");
        assert_eq!(lut, Lut::identity(2));
        assert_eq!(&lut.rgba8()[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn broken_cube_files_say_why() {
        assert_eq!(parse_cube("0 0 0\n"), Err(CubeError::NoSize));
        assert_eq!(parse_cube("LUT_1D_SIZE 16\n"), Err(CubeError::NoSize));
        assert_eq!(parse_cube("LUT_3D_SIZE 1\n"), Err(CubeError::Size(1)));
        assert_eq!(parse_cube("LUT_3D_SIZE 2\n0 0\n"), Err(CubeError::Line(2)));
        assert_eq!(
            parse_cube("LUT_3D_SIZE 2\n0 0 0\n"),
            Err(CubeError::Count { found: 1, expected: 8 })
        );
    }
}
