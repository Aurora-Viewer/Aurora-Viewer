//! The 360° snapshot's rules (LLFloater360Capture, indra/newview/
//! llfloater360capture.cpp, originally LGPL 2.1): its quality levels (the
//! side of the six faces), the heading written in the image, the file name
//! proposed, and the JPEG's XMP block (the GPano fields the Legacy Viewer's
//! stitching page writes, so that photo sites show the file as a panorama).
//! The capture itself is `app/capture_360.rs`, its controls the photo window's
//! 360° format (`ui/snapshot.rs`), the projection `aurora_render::equirect`,
//! the viewer `aurora_render::panorama` with `ui/panorama.rs`.

use serde::{Deserialize, Serialize};

/// The width the Legacy Viewer writes (360CaptureOutputImageWidth), the
/// fallback of a card that cannot make the panorama of the quality chosen.
pub const OUTPUT_WIDTH: u32 = 4096;
/// The faces of a panorama uploaded to the inventory: four of them across
/// the equator make the 2048 pixels of the largest texture the grid takes.
pub const INVENTORY_FACE: u32 = 512;
/// The texture an inventory panorama is (2048 is the upload's largest side).
pub const INVENTORY_OUTPUT: (u32, u32) = (2048, 1024);
/// JPEG quality of the saved panorama (360CaptureJPEGEncodeQuality).
pub const JPEG_QUALITY: u8 = 95;
/// What an Aurora 360° upload's item name starts with. ASCII only: the grid
/// (and the Legacy Viewer, `LLStringUtil::replaceNonstandardASCII` through
/// `LLInventoryObject::correctInventoryName`) replace every byte above 0x7f
/// in an item name, so a "°" would not come back.
pub const MARKER: &str = "360 Snapshot";
/// Longest item name the grid keeps (DB_INV_ITEM_NAME_STR_LEN).
const NAME_LIMIT: usize = 63;

/// The XMP packet's GPano version written by the Legacy Viewer.
const PANO_VERSION: &str = "2.2.1";
/// How many faces a capture is made of (GPano:SourcePhotosCount).
const SOURCE_PHOTOS: u32 = 6;

/// The quality levels (floater_360capture.xml's radio group, the value of
/// each being the side of the square faces; 360QualitySelection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    Preview,
    Medium,
    High,
    Maximum,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Preview, Quality::Medium, Quality::High, Quality::Maximum];

    /// Side of each cube face in pixels.
    pub fn face_size(self) -> u32 {
        match self {
            Quality::Preview => 128,
            Quality::Medium => 512,
            Quality::High => 1024,
            Quality::Maximum => 2048,
        }
    }

    /// The radio's label.
    pub fn label_key(self) -> &'static str {
        match self {
            Quality::Preview => "capture-360-quality-preview",
            Quality::Medium => "capture-360-quality-medium",
            Quality::High => "capture-360-quality-high",
            Quality::Maximum => "capture-360-quality-maximum",
        }
    }

    /// The panorama's size this quality asks for: four faces across the
    /// equator, so that every pixel of a face is used. The Legacy Viewer
    /// writes 4096 whatever the faces (a visible difference: Preview is
    /// smaller, Maximum larger).
    pub fn output(self) -> (u32, u32) {
        let width = self.face_size() * 4;
        (width, width / 2)
    }

    /// The level named by a word of a demo scenario.
    pub fn from_word(word: &str) -> Option<Quality> {
        match word {
            "preview" => Some(Quality::Preview),
            "medium" => Some(Quality::Medium),
            "high" => Some(Quality::High),
            "maximum" => Some(Quality::Maximum),
            _ => None,
        }
    }
}

/// The window's settings, kept between sessions (360QualitySelection,
/// 360CaptureHideAvatars).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub quality: Quality,
    pub hide_avatars: bool,
}

/// The panorama's size for faces of `face_size` on a card whose textures go up
/// to `max_texture`: four faces across, or 4096 (the Legacy Viewer's) when
/// the card cannot make that, or the card's own limit when it cannot even do
/// that. Width over height is 2.
pub fn output_size(face_size: u32, max_texture: u32) -> (u32, u32) {
    let wanted = face_size.saturating_mul(4);
    let width = if wanted <= max_texture {
        wanted
    } else if OUTPUT_WIDTH <= max_texture {
        OUTPUT_WIDTH
    } else {
        max_texture
    };
    // (an even width, for the half height)
    let width = (width & !1).max(2);
    (width, width / 2)
}

/// The panorama turned so that the compass heading `heading` (degrees, north
/// 0, east 90) is its middle column. The XMP block that carries the heading
/// does not survive the texture upload, so the inventory's picture is
/// rolled instead: a viewer that opens a panorama at its middle looks where
/// the camera did. `rgba` is `width` x `height`, rows top to bottom.
pub fn roll_to_heading(rgba: &[u8], width: u32, height: u32, heading: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || rgba.len() != w * h * 4 {
        return rgba.to_vec();
    }
    // the heading's column is at (heading / 360 + 0.5) of the width; the
    // middle column takes its place
    let shift = ((f64::from(heading % 360) / 360.0) * w as f64).round() as usize % w;
    let mut out = Vec::with_capacity(rgba.len());
    for row in rgba.chunks_exact(w * 4) {
        out.extend_from_slice(&row[shift * 4..]);
        out.extend_from_slice(&row[..shift * 4]);
    }
    out
}

/// The compass heading the camera looks at, as the XMP's
/// GPano:InitialViewHeadingDegrees wants it (N = 0, E = 90, S = 180,
/// W = 270): `(360 + 90 - yaw°) % 360` of LLCamera::getYaw, the angle of
/// the view in the ground plane from +X.
pub fn heading_degrees(forward: glam::Vec3) -> u32 {
    let yaw = forward.y.atan2(forward.x).to_degrees();
    // (the Legacy Viewer truncates the yaw to whole degrees first)
    (360 + 90 - yaw as i32).rem_euclid(360) as u32
}

/// LLFloater360Capture::generate_proposed_filename: `sl360_<region>_
/// <width>x<height>_<face>_<stamp>.jpg`, the region's name with every
/// character that is not a letter or a digit replaced by `_`.
pub fn proposed_file_name(region: &str, width: u32, height: u32, face_size: u32, stamp: &str) -> String {
    let mut name = String::from("sl360_");
    if !region.is_empty() {
        name.extend(region.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }));
        name.push('_');
    }
    format!("{name}{width}x{height}_{face_size}_{stamp}.jpg")
}

/// The inventory item's name of an uploaded 360° picture, after the Legacy
/// Viewer's "Snapshot : <place>": `360 Snapshot : <place>`, clipped to the
/// grid's 63 bytes. The place is cleaned to ASCII, the marker is never cut.
pub fn item_name(place: &str) -> String {
    let place: String = place
        .chars()
        .map(|c| if c.is_ascii() && !c.is_control() { c } else { '_' })
        .collect();
    let mut name = format!("{MARKER} : {place}");
    name.truncate(NAME_LIMIT);
    name.trim_end().to_owned()
}

/// A texture an Aurora 360° upload made: its name starts with the marker
/// (any case, after any leading spaces). Whether it is a panorama still
/// depends on its shape (`is_equirect`).
pub fn is_marked(name: &str) -> bool {
    name.trim_start()
        .get(..MARKER.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(MARKER))
}

/// An equirectangular panorama covers 360° by 180°: twice as wide as high.
/// Smaller than 256 wide, it is too small to look around in.
pub fn is_equirect((width, height): (u32, u32)) -> bool {
    width >= 256 && width == height.saturating_mul(2)
}

/// What the XMP block says of a panorama (the `xmp_details` the Legacy
/// Viewer hands its stitching page).
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    /// The viewer's name and version (the channel and the short version).
    pub software: String,
    pub width: u32,
    pub height: u32,
    pub heading: u32,
    /// The face size chosen, and the one drawn (smaller on a card that
    /// cannot draw the chosen one).
    pub source_size: u32,
    pub scaled_source_size: u32,
    /// The capture's local date and time (std::ctime's form).
    pub date: String,
    pub region_name: String,
    pub region_url: String,
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The XMP packet of the panorama: Google's photo sphere fields
/// (GPano), which Facebook, Flickr and the photo viewers read to show the
/// file as a 360° image, the viewer as the creator tool, and the place in
/// Dublin Core (the region's name and its SLURL).
pub fn xmp_packet(m: &Meta) -> String {
    let software = escape_xml(&m.software);
    let region = escape_xml(&m.region_name);
    let url = escape_xml(&m.region_url);
    let date = escape_xml(&m.date);
    format!(
        concat!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n",
            " <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n",
            "  <rdf:Description rdf:about=\"\"\n",
            "    xmlns:GPano=\"http://ns.google.com/photos/1.0/panorama/\"\n",
            "    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n",
            "    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n",
            "    GPano:ProjectionType=\"equirectangular\"\n",
            "    GPano:UsePanoramaViewer=\"True\"\n",
            "    GPano:PanoVersion=\"{pano_version}\"\n",
            "    GPano:CaptureSoftware=\"{software}\"\n",
            "    GPano:StitchingSoftware=\"{software}\"\n",
            "    GPano:SourcePhotosCount=\"{photos}\"\n",
            "    GPano:FullPanoWidthPixels=\"{width}\"\n",
            "    GPano:FullPanoHeightPixels=\"{height}\"\n",
            "    GPano:CroppedAreaImageWidthPixels=\"{width}\"\n",
            "    GPano:CroppedAreaImageHeightPixels=\"{height}\"\n",
            "    GPano:CroppedAreaLeftPixels=\"0\"\n",
            "    GPano:CroppedAreaTopPixels=\"0\"\n",
            "    GPano:InitialViewHeadingDegrees=\"{heading}\"\n",
            "    GPano:FirstPhotoDate=\"{date}\"\n",
            "    GPano:LastPhotoDate=\"{date}\"\n",
            "    GPano:SourceImageSize=\"{source}\"\n",
            "    GPano:ScaledSourceImageSize=\"{scaled}\"\n",
            "    xmp:CreatorTool=\"{software}\"\n",
            "    dc:description=\"{region}\"\n",
            "    dc:source=\"{url}\"/>\n",
            " </rdf:RDF>\n",
            "</x:xmpmeta>\n",
            "<?xpacket end=\"w\"?>"
        ),
        pano_version = PANO_VERSION,
        software = software,
        photos = SOURCE_PHOTOS,
        width = m.width,
        height = m.height,
        heading = m.heading,
        date = date,
        source = m.source_size,
        scaled = m.scaled_source_size,
        region = region,
        url = url,
    )
}

/// The namespace that opens a JPEG's XMP segment (XMP specification part
/// 3: APP1, this string, NUL, the packet).
const XMP_APP1_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// `jpeg` with `xmp` as an APP1 segment after its JFIF segment (or right
/// after the start of image when it has none). An XMP too long for one
/// segment, or something that is not a JPEG, comes back unchanged.
pub fn insert_xmp(jpeg: &[u8], xmp: &str) -> Vec<u8> {
    let payload_len = XMP_APP1_HEADER.len() + xmp.len() + 2;
    if jpeg.len() < 4 || jpeg[0..2] != [0xFF, 0xD8] || payload_len > 0xFFFF {
        return jpeg.to_vec();
    }
    // after the SOI, past an APP0 (JFIF) segment if there is one
    let mut at = 2;
    if jpeg.len() >= 6 && jpeg[2..4] == [0xFF, 0xE0] {
        let len = usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
        at = (4 + len).min(jpeg.len());
    }
    let mut out = Vec::with_capacity(jpeg.len() + payload_len + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.extend_from_slice(XMP_APP1_HEADER);
    out.extend_from_slice(xmp.as_bytes());
    out.extend_from_slice(&jpeg[at..]);
    out
}

/// The panorama as the file's bytes: a JPEG at the Legacy Viewer's quality
/// with the XMP block of `meta`.
pub fn encode(width: u32, height: u32, rgba: &[u8], meta: &Meta) -> Result<Vec<u8>, String> {
    let jpeg = super::save::encode_jpeg(width, height, rgba, JPEG_QUALITY)?;
    Ok(insert_xmp(&jpeg, &xmp_packet(meta)))
}

/// std::ctime's form of a local date and time, without its newline (the
/// photo dates the Legacy Viewer writes): `Sat Nov  7 14:03:05 2026`.
pub fn ctime_stamp((year, month, day, hour, minute, second): (u16, u16, u16, u16, u16, u16)) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let weekday = DAYS[weekday(i32::from(year), u32::from(month), u32::from(day))];
    let month = MONTHS[usize::from(month.clamp(1, 12)) - 1];
    format!("{weekday} {month} {day:2} {hour:02}:{minute:02}:{second:02} {year}")
}

/// `%Y%m%d_%H%M%S` of a local date and time (the proposed file name).
pub fn compact_stamp((year, month, day, hour, minute, second): (u16, u16, u16, u16, u16, u16)) -> String {
    format!("{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}")
}

/// Day of the week of a civil date, 0 Sunday (Zeller's congruence).
fn weekday(year: i32, month: u32, day: u32) -> usize {
    let (y, m) = if month < 3 { (year - 1, month + 12) } else { (year, month) };
    let (k, j) = (y.rem_euclid(100), y.div_euclid(100));
    let h = (day as i32 + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    // Zeller's 0 is Saturday
    ((h + 6) % 7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn headings_follow_the_compass() {
        assert_eq!(heading_degrees(Vec3::Y), 0, "north");
        assert_eq!(heading_degrees(Vec3::X), 90, "east");
        assert_eq!(heading_degrees(Vec3::NEG_Y), 180, "south");
        assert_eq!(heading_degrees(Vec3::NEG_X), 270, "west");
        // a camera looking at the sky still has a heading
        assert_eq!(heading_degrees(Vec3::new(0.0, 0.3, 0.95)), 0);
    }

    #[test]
    fn proposed_name_cleans_the_region() {
        assert_eq!(
            proposed_file_name("Aurora Démo (2)", 4096, 2048, 512, "20261107_140305"),
            "sl360_Aurora_D_mo__2__4096x2048_512_20261107_140305.jpg"
        );
        assert_eq!(proposed_file_name("", 4096, 2048, 128, "s"), "sl360_4096x2048_128_s.jpg");
    }

    fn meta() -> Meta {
        Meta {
            software: "Aurora Viewer 0.5.0".into(),
            width: 4096,
            height: 2048,
            heading: 90,
            source_size: 512,
            scaled_source_size: 512,
            date: "Sat Nov  7 14:03:05 2026".into(),
            region_name: "A & B".into(),
            region_url: "http://maps.secondlife.com/secondlife/A%20%26%20B/1/2/3".into(),
        }
    }

    #[test]
    fn xmp_names_the_panorama_and_escapes_the_place() {
        let xmp = xmp_packet(&meta());
        assert!(xmp.contains("GPano:ProjectionType=\"equirectangular\""));
        assert!(xmp.contains("GPano:FullPanoWidthPixels=\"4096\""));
        assert!(xmp.contains("GPano:FullPanoHeightPixels=\"2048\""));
        assert!(xmp.contains("GPano:InitialViewHeadingDegrees=\"90\""));
        assert!(xmp.contains("dc:description=\"A &amp; B\""));
        assert!(xmp.contains("GPano:SourcePhotosCount=\"6\""));
        assert!(xmp.starts_with("<?xpacket begin="));
        assert!(xmp.ends_with("<?xpacket end=\"w\"?>"));
    }

    #[test]
    fn xmp_goes_after_the_jfif_segment() {
        // SOI, a 16-byte APP0, a marker that follows
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        jpeg.extend_from_slice(b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0");
        jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x02]);
        let out = insert_xmp(&jpeg, "<x/>");
        assert_eq!(&out[..20], &jpeg[..20], "SOI and JFIF kept first");
        assert_eq!(&out[20..22], &[0xFF, 0xE1]);
        let len = usize::from(u16::from_be_bytes([out[22], out[23]]));
        assert_eq!(len, 2 + XMP_APP1_HEADER.len() + 4);
        assert_eq!(&out[24..24 + XMP_APP1_HEADER.len()], XMP_APP1_HEADER);
        assert_eq!(&out[out.len() - 4..], &[0xFF, 0xDB, 0x00, 0x02]);
        // without JFIF: right after the SOI
        let bare = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x02];
        let out = insert_xmp(&bare, "<x/>");
        assert_eq!(&out[2..4], &[0xFF, 0xE1]);
        // not a JPEG: untouched
        assert_eq!(insert_xmp(b"PNG", "<x/>"), b"PNG");
    }

    #[test]
    fn encoded_file_is_a_jpeg_with_its_xmp() {
        let rgba = vec![200u8; 8 * 4 * 4];
        let bytes = encode(8, 4, &rgba, &meta()).expect("a JPEG");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        let packet = xmp_packet(&meta());
        assert!(bytes.windows(packet.len()).any(|w| w == packet.as_bytes()));
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn stamps_follow_ctime_and_the_file_name() {
        assert_eq!(ctime_stamp((2026, 11, 7, 14, 3, 5)), "Sat Nov  7 14:03:05 2026");
        assert_eq!(ctime_stamp((2000, 1, 1, 0, 0, 0)), "Sat Jan  1 00:00:00 2000");
        assert_eq!(ctime_stamp((2024, 2, 29, 23, 59, 59)), "Thu Feb 29 23:59:59 2024");
        assert_eq!(compact_stamp((2026, 11, 7, 14, 3, 5)), "20261107_140305");
    }

    #[test]
    fn the_marker_is_ascii_and_survives_a_long_place_name() {
        let name = item_name("A place with a very long name that goes on and on and on, far past the limit of an item");
        assert!(name.is_ascii(), "the grid replaces every non-ASCII byte of a name");
        assert!(name.len() <= 63);
        assert!(is_marked(&name));
        assert!(MARKER.is_ascii());
        assert!(item_name("Démo ° (2)").is_ascii());
        assert!(is_marked(&item_name("Démo ° (2)")));
        assert_eq!(item_name("Aurora"), "360 Snapshot : Aurora");
    }

    #[test]
    fn marked_names_are_found_whatever_the_case_or_the_spaces() {
        for name in ["360 Snapshot : Aurora", "360 snapshot", "  360 SNAPSHOT : x", "360 Snapshot"] {
            assert!(is_marked(name), "{name}");
        }
        for name in ["Snapshot : Aurora", "My 360 Snapshot", "", "360", "360 Snap", "360 Snapshé"] {
            assert!(!is_marked(name), "{name}");
        }
    }

    #[test]
    fn only_two_to_one_textures_of_enough_width_are_panoramas() {
        assert!(is_equirect((2048, 1024)));
        assert!(is_equirect((256, 128)));
        assert!(!is_equirect((128, 64)), "too small to look around in");
        assert!(!is_equirect((1024, 1024)));
        assert!(!is_equirect((2048, 1025)));
        assert!(!is_equirect((0, 0)));
    }

    #[test]
    fn the_panorama_has_four_faces_across_the_equator() {
        assert_eq!(
            Quality::ALL.map(Quality::output),
            [(512, 256), (2048, 1024), (4096, 2048), (8192, 4096)]
        );
        // a card that takes them all
        assert_eq!(output_size(2048, 16384), (8192, 4096));
        assert_eq!(output_size(128, 16384), (512, 256));
        // a card limited to 4096: the Legacy Viewer's width
        assert_eq!(output_size(2048, 4096), (4096, 2048));
        assert_eq!(output_size(1024, 4096), (4096, 2048));
        // a card that cannot make even that
        assert_eq!(output_size(2048, 2048), (2048, 1024));
        assert_eq!(output_size(2048, 2047), (2046, 1023));
        // the inventory's: 4 x 512 = 2048 wide, the texture's size
        assert_eq!(output_size(INVENTORY_FACE, 16384), INVENTORY_OUTPUT);
    }

    /// A column marked at the heading comes to the middle; nothing else is
    /// lost, and a whole turn changes nothing.
    #[test]
    fn rolling_puts_the_heading_in_the_middle_column() {
        let (w, h) = (8u32, 2u32);
        let image = |marked: usize| {
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for row in 0..h as usize {
                rgba[(row * w as usize + marked) * 4] = 255;
            }
            rgba
        };
        // north is the middle already
        assert_eq!(roll_to_heading(&image(4), w, h, 0), image(4));
        // east, 90 degrees, a quarter of the width right of the middle
        assert_eq!(roll_to_heading(&image(6), w, h, 90), image(4));
        // west is left of the middle
        assert_eq!(roll_to_heading(&image(2), w, h, 270), image(4));
        // south wraps round to the edge
        assert_eq!(roll_to_heading(&image(0), w, h, 180), image(4));
        assert_eq!(roll_to_heading(&image(4), w, h, 360), image(4));
        // a malformed image comes back as it is
        assert_eq!(roll_to_heading(&[1, 2, 3], w, h, 90), vec![1, 2, 3]);
    }

    #[test]
    fn quality_levels_keep_the_legacy_sizes() {
        assert_eq!(Quality::ALL.map(Quality::face_size), [128, 512, 1024, 2048]);
        assert_eq!(Quality::from_word("high"), Some(Quality::High));
        assert_eq!(Settings::default().quality, Quality::Preview);
    }
}
