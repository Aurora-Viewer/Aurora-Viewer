//! Abuse reports: the rules of the "Report Abuse" window, ported from
//! LLFloaterReporter (indra/newview/llfloaterreporter.cpp and
//! floater_report_abuse.xml, originally LGPL 2.1): its categories, what
//! must be filled before sending, the copyright warning, and the summary and
//! details texts the report carries. The window is `ui::report_abuse`, its
//! wiring `app::report_abuse`, the network side `aurora_net::abuse_report`.

use glam::Vec3;

/// What a report is opened on.
#[derive(Debug, Clone, PartialEq)]
pub enum ReportTarget {
    /// Help › Report Abuse (showFromMenu).
    Nothing,
    /// A resident of a list or a profile (showFromAvatar).
    Avatar(uuid::Uuid),
    /// What was clicked in the world, avatar or object (showFromObject).
    Object(uuid::Uuid),
    /// A chat line (showFromChat): its speaker, time and text.
    Chat { avatar: uuid::Uuid, time: String, text: String },
    /// An experience's profile (showFromExperience): its owner is the
    /// abuser, the details name the experience.
    Experience(uuid::Uuid),
}

/// getExperienceInfo's details: the experience's id, or that its details
/// could not be read (sent to the grid as they are, in English, like the
/// other lines gatherReport writes).
pub fn experience_details(experience: uuid::Uuid, known: bool) -> String {
    if known {
        format!("Experience id: {experience}")
    } else {
        format!("Unable to retrieve details for id: {experience}")
    }
}

/// The categories of floater_report_abuse.xml, kept until the region's
/// AbuseCategories answer replaces them: (value, label key).
pub const CATEGORIES: [(i32, &str); 14] = [
    (31, "report-abuse-category-31"),
    (35, "report-abuse-category-35"),
    (39, "report-abuse-category-39"),
    (43, "report-abuse-category-43"),
    (44, "report-abuse-category-44"),
    (45, "report-abuse-category-45"),
    (50, "report-abuse-category-50"),
    (55, "report-abuse-category-55"),
    (57, "report-abuse-category-57"),
    (59, "report-abuse-category-59"),
    (60, "report-abuse-category-60"),
    (61, "report-abuse-category-61"),
    (63, "report-abuse-category-63"),
    (67, "report-abuse-category-67"),
];

/// The built-in list, labels in the viewer's language ("Select category",
/// value 0, is the window's placeholder, not an entry).
pub fn default_categories() -> Vec<(i32, String)> {
    CATEGORIES
        .iter()
        .map(|(value, key)| (*value, aurora_i18n::text(key).to_owned()))
        .collect()
}

/// summary_edit max_length (bytes).
pub const SUMMARY_MAX_BYTES: usize = 64;
/// abuse_location_edit max_length_bytes.
pub const LOCATION_MAX_BYTES: usize = 256;
/// details_edit max_length (characters).
pub const DETAILS_MAX_CHARS: usize = 800;
/// IP_CONTENT_REMOVAL: always answered by the copyright notice, never sent.
pub const IP_CONTENT_REMOVAL: i32 = 66;
/// IP_PERMISSONS_EXPLOIT.
pub const IP_PERMISSIONS_EXPLOIT: i32 = 37;
/// AbuseReportScreenshotDelay: the window waits this long before its
/// screenshot ("to avoid UI artifacts", MAINT-7067).
pub const SCREENSHOT_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// What the user typed or chose, as the window's fields hold it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    /// 0: "Select category".
    pub category: i32,
    /// The label of the chosen category (the report carries the label).
    pub category_label: String,
    pub abuser_name: String,
    pub location: String,
    pub summary: String,
    pub details: String,
    /// "Object:" and "Owner:" of the picked object (an avatar: its name in
    /// both).
    pub object_name: String,
    pub owner_name: String,
}

/// The notice a click on "Report Abuse" answers with instead of sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    NeedCategory,
    NeedAbuser,
    NeedLocation,
    NeedSummary,
    NeedDetails,
    /// HelpReportAbuseContainsCopyright.
    Copyright,
}

impl Notice {
    pub fn text(self, grid: &str) -> String {
        match self {
            Notice::NeedCategory => aurora_i18n::tr!("report-abuse-need-category").into(),
            Notice::NeedAbuser => aurora_i18n::tr!("report-abuse-need-abuser").into(),
            Notice::NeedLocation => aurora_i18n::tr!("report-abuse-need-location").into(),
            Notice::NeedSummary => aurora_i18n::tr!("report-abuse-need-summary").into(),
            Notice::NeedDetails => aurora_i18n::tr!("report-abuse-need-details").into(),
            Notice::Copyright => aurora_i18n::tr!("report-abuse-copyright", grid = grid.to_string()),
        }
    }
}

/// validateReport, in its order. `default_details` is what the details held
/// when the window opened (mDefaultSummary): unchanged counts as empty.
pub fn validate(form: &Form, default_details: &str) -> Option<Notice> {
    if form.category == 0 {
        Some(Notice::NeedCategory)
    } else if form.abuser_name.is_empty() {
        Some(Notice::NeedAbuser)
    } else if form.location.is_empty() {
        Some(Notice::NeedLocation)
    } else if form.summary.is_empty() {
        Some(Notice::NeedSummary)
    } else if form.details == default_details {
        Some(Notice::NeedDetails)
    } else {
        None
    }
}

/// onClickSend's copyright check, after validation: the first time a
/// report mentions "copyright" or is about intellectual property, the
/// notice is shown and the report is not sent; sending again goes, except
/// for IP_CONTENT_REMOVAL which never does.
pub fn copyright_notice(form: &Form, seen: &mut bool) -> bool {
    if !*seen {
        let mentions = |text: &str| text.to_lowercase().contains("copyright");
        if mentions(&form.details)
            || mentions(&form.summary)
            || form.category == IP_CONTENT_REMOVAL
            || form.category == IP_PERMISSIONS_EXPLOIT
        {
            *seen = true;
            return true;
        }
        false
    } else {
        form.category == IP_CONTENT_REMOVAL
    }
}

/// gatherReport's summary: `Preview` on the beta grid, then
/// ` |Region| (location) [category]  {abuser}  "summary"`.
pub fn summary(form: &Form, region: &str, beta_grid: bool) -> String {
    format!(
        "{} |{region}| ({}) [{}]  {{{}}}  \"{}\"",
        if beta_grid { "Preview " } else { "" },
        form.location,
        form.category_label,
        form.abuser_name,
        form.summary
    )
}

/// gatherReport's details: the viewer version, the object and its owner
/// when one was picked, the abuser and the place, then what was typed.
pub fn details(form: &Form, version: &str) -> String {
    let mut s = format!("V{version}\n\n");
    if !form.object_name.is_empty() && !form.owner_name.is_empty() {
        s += &format!("Object: {}\nOwner: {}\n", form.object_name, form.owner_name);
    }
    s += &format!("Abuser name: {} \n", form.abuser_name);
    s += &format!("Abuser location: {} \n", form.location);
    s += &form.details;
    s
}

/// The report's VersionString: "version platform CPU renderer driver".
pub fn version_string(version: &str, cpu: &str, renderer: &str, driver: &str) -> String {
    let platform = if cfg!(windows) {
        "Win"
    } else if cfg!(target_os = "macos") {
        "Mac"
    } else if cfg!(target_os = "linux") {
        "Lnx"
    } else {
        "???"
    };
    format!("{version} {platform} {cpu} {renderer} {driver}")
}

/// setPosBox: `{128.1, 128.1, 15.4}`.
pub fn position_text(position: Vec3) -> String {
    format!("{{{:.1}, {:.1}, {:.1}}}", position.x, position.y, position.z)
}

/// The report of a chat line's sender: its time (SLT,
/// "[month]/[day]/[year] [hour]:[min]") and its text.
pub fn chat_target(avatar: uuid::Uuid, line: &crate::world::ChatLine) -> ReportTarget {
    let time = if line.time == std::time::UNIX_EPOCH {
        String::new()
    } else {
        let t = crate::world::chat_log::format::Slt::of(line.time);
        format!("{:02}/{:02}/{} {:02}:{:02}", t.month, t.day, t.year, t.hour, t.minute)
    };
    ReportTarget::Chat {
        avatar,
        time,
        text: line.text.clone(),
    }
}

/// showFromChat: the details of a report made on a chat line.
pub fn chat_details(time: &str, text: &str) -> String {
    aurora_i18n::tr!("report-abuse-chat-format", time = time.to_string(), text = text.to_string())
}

/// takeNewSnapshot's size: the window's picture fitted in 1024 × 768.
pub fn screenshot_size(window: (u32, u32)) -> (u32, u32) {
    let (w, h) = (window.0.max(1) as f32, window.1.max(1) as f32);
    let scale = (1024.0 / w).min(768.0 / h).min(1.0);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// The screenshot of a report, off the main thread: its preview and its
/// JPEG 2000 texture, scaled to powers of two like any uploaded texture
/// (takeScreenshot: LLViewerTextureList::convertToUploadFile). None for
/// the texture when it cannot be encoded: the report goes without it.
pub fn prepare_screenshot(width: u32, height: u32, rgba: Vec<u8>) -> Option<(egui::ColorImage, Option<Vec<u8>>)> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let (w, h) = screenshot_size((width, height));
    let image = if (w, h) == (width, height) {
        image
    } else {
        image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle)
    };
    let preview = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], image.as_raw());
    let texture = crate::snapshot::inventory::texture_size((w, h));
    let j2c = match crate::snapshot::inventory::encode(w, h, image.as_raw(), texture) {
        Ok(data) => Some(data),
        Err(e) => {
            log::warn!("abuse report: screenshot not encoded: {e}");
            None
        }
    };
    Some((preview, j2c))
}

/// `text` cut to `max` bytes, on a character (the line editors' limits).
pub fn clamp_bytes(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// `text` cut to `max` characters.
pub fn clamp_chars(text: &mut String, max: usize) {
    if let Some((end, _)) = text.char_indices().nth(max) {
        text.truncate(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Form {
        Form {
            category: 45,
            category_label: "Disturbing the peace > Repetitive spam".into(),
            abuser_name: "Nova Exemple".into(),
            location: "http://maps.secondlife.com/secondlife/Plaza/128/64/22".into(),
            summary: "Spam".into(),
            details: "Spams the nearby chat.".into(),
            ..Default::default()
        }
    }

    #[test]
    fn every_field_is_checked_in_firestorms_order() {
        let mut form = filled();
        assert_eq!(validate(&form, ""), None);
        form.details.clear();
        assert_eq!(validate(&form, ""), Some(Notice::NeedDetails));
        // a chat report starts with its details: left as they are, they count as empty
        form.details = "Time: 12:00".into();
        assert_eq!(validate(&form, "Time: 12:00"), Some(Notice::NeedDetails));
        assert_eq!(validate(&form, ""), None);
        form.summary.clear();
        assert_eq!(validate(&form, ""), Some(Notice::NeedSummary));
        form.location.clear();
        assert_eq!(validate(&form, ""), Some(Notice::NeedLocation));
        form.abuser_name.clear();
        assert_eq!(validate(&form, ""), Some(Notice::NeedAbuser));
        form.category = 0;
        assert_eq!(validate(&form, ""), Some(Notice::NeedCategory));
    }

    #[test]
    fn copyright_is_warned_once_and_content_removal_is_never_sent() {
        let mut seen = false;
        let mut form = filled();
        assert!(!copyright_notice(&form, &mut seen));
        form.details = "He sells my COPYRIGHTED textures".into();
        assert!(copyright_notice(&form, &mut seen));
        assert!(!copyright_notice(&form, &mut seen), "the second click sends");
        let mut seen = false;
        form = filled();
        form.category = IP_PERMISSIONS_EXPLOIT;
        assert!(copyright_notice(&form, &mut seen));
        assert!(!copyright_notice(&form, &mut seen));
        let mut seen = false;
        form.category = IP_CONTENT_REMOVAL;
        assert!(copyright_notice(&form, &mut seen));
        assert!(copyright_notice(&form, &mut seen), "always the notice");
    }

    #[test]
    fn summary_and_details_are_those_of_gather_report() {
        let mut form = filled();
        assert_eq!(
            summary(&form, "Plaza", false),
            " |Plaza| (http://maps.secondlife.com/secondlife/Plaza/128/64/22) [Disturbing the peace > Repetitive spam]  {Nova Exemple}  \"Spam\""
        );
        assert!(summary(&form, "Plaza", true).starts_with("Preview  |Plaza| "));
        assert_eq!(
            details(&form, "0.4.0"),
            "V0.4.0\n\nAbuser name: Nova Exemple \nAbuser location: http://maps.secondlife.com/secondlife/Plaza/128/64/22 \nSpams the nearby chat."
        );
        form.object_name = "Spammer box".into();
        form.owner_name = "nova.exemple".into();
        assert!(details(&form, "0.4.0").starts_with("V0.4.0\n\nObject: Spammer box\nOwner: nova.exemple\nAbuser name: "));
        assert_eq!(position_text(Vec3::new(128.06, 64.0, 22.44)), "{128.1, 64.0, 22.4}");
        assert!(version_string("0.4.0", "CPU", "GPU", "1.2").starts_with("0.4.0 "));
    }

    #[test]
    fn an_experience_report_names_the_experience_in_its_details() {
        let id = uuid::Uuid::from_u128(0xE);
        assert_eq!(experience_details(id, true), format!("Experience id: {id}"));
        assert_eq!(experience_details(id, false), format!("Unable to retrieve details for id: {id}"));
    }

    #[test]
    fn the_built_in_categories_are_firestorms() {
        let list = default_categories();
        assert_eq!(list.len(), 14);
        assert_eq!(list.first().map(|c| c.0), Some(31));
        assert_eq!(list.last().map(|c| c.0), Some(67));
        assert!(list.iter().all(|(_, label)| !label.is_empty()));
    }

    #[test]
    fn the_screenshot_fits_in_1024_by_768_as_a_power_of_two_texture() {
        assert_eq!(screenshot_size((1920, 1080)), (1024, 576));
        assert_eq!(screenshot_size((1000, 1000)), (768, 768));
        assert_eq!(screenshot_size((800, 600)), (800, 600));
        let (preview, j2c) = prepare_screenshot(64, 32, vec![128; 64 * 32 * 4]).expect("pixels");
        assert_eq!(preview.size, [64, 32]);
        assert!(j2c.is_some_and(|d| d.starts_with(&[0xFF, 0x4F])));
        assert!(prepare_screenshot(64, 32, vec![0; 10]).is_none());
    }

    #[test]
    fn fields_are_cut_like_their_editors() {
        let mut text = "é".repeat(40);
        clamp_bytes(&mut text, SUMMARY_MAX_BYTES);
        assert_eq!(text.len(), 64);
        let mut text = "é".repeat(900);
        clamp_chars(&mut text, DETAILS_MAX_CHARS);
        assert_eq!(text.chars().count(), 800);
    }
}
