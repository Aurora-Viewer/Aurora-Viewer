//! Abuse reports: the report LLSD, its categories and how it is sent.
//! Port of LLFloaterReporter::gatherReport, sendReportViaCaps,
//! sendReportViaLegacy and requestAbuseCategoriesCoro, and of the upload
//! LLARScreenShotUploader goes through (LLViewerAssetUpload::
//! AssetInventoryUploadCoproc) (indra/newview/llfloaterreporter.cpp and
//! llviewerassetupload.cpp, originally LGPL 2.1).

use aurora_llsd::{Llsd, llsd_map};
use glam::Vec3;
use uuid::Uuid;

/// COMPLAINT_REPORT (EReportType): the only kind the floater sends.
pub const COMPLAINT_REPORT: u8 = 2;
/// COMPLAINT_REPORT_REQUEST: RequestFlags of the RequestObjectPropertiesFamily
/// sent for the object the report is about (llfloaterreporter.h).
pub const COMPLAINT_REPORT_REQUEST: u32 = 1 << 1;
/// LL_ASSET_UPLOAD_TIMEOUT_SEC.
const UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 5);

/// What the floater gathered (gatherReport), before the screenshot is known.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AbuseReport {
    pub category: i32,
    /// Region-local position the report was made at (mPosition).
    pub position: Vec3,
    pub object_id: Uuid,
    pub abuser_id: Uuid,
    /// " |Region| (location) [category] {abuser}  \"summary\"".
    pub summary: String,
    pub details: String,
    pub version_string: String,
    /// The screenshot, encoded as a JPEG 2000 texture; None without one.
    pub screenshot: Option<Vec<u8>>,
}

impl AbuseReport {
    /// The LLSD of gatherReport, field for field.
    pub fn body(&self, screenshot_id: Uuid) -> Llsd {
        llsd_map! {
            "report-type" => i32::from(COMPLAINT_REPORT),
            "category" => self.category,
            "position" => vec![
                Llsd::from(f64::from(self.position.x)),
                Llsd::from(f64::from(self.position.y)),
                Llsd::from(f64::from(self.position.z)),
            ],
            // "this is not used"
            "check-flags" => 0,
            "screenshot-id" => screenshot_id,
            "object-id" => self.object_id,
            "abuser-id" => self.abuser_id,
            "abuse-region-name" => "",
            "abuse-region-id" => Uuid::nil(),
            "summary" => self.summary.as_str(),
            "version-string" => self.version_string.as_str(),
            "details" => self.details.as_str(),
        }
    }
}

/// The AbuseCategories capability in the viewer's language
/// (`?lc=<Language>` unless it is "default").
pub fn categories_url(cap: &str, language: &str) -> String {
    if language.is_empty() || language == "default" {
        cap.to_owned()
    } else {
        format!("{cap}?lc={language}")
    }
}

/// `categories`: `{category, description_localized}` in the server's
/// order; None when the answer has none (the floater keeps its list).
pub fn parse_categories(reply: &Llsd) -> Option<Vec<(i32, String)>> {
    if !reply.has("categories") {
        return None;
    }
    let list: Vec<(i32, String)> = reply["categories"]
        .as_array()
        .iter()
        .map(|c| (c["category"].as_i32(), c["description_localized"].to_string_value()))
        .filter(|(value, label)| *value != 0 && !label.trim().is_empty())
        .collect();
    (!list.is_empty()).then_some(list)
}

/// LLTransactionID::makeAssetID: the id the screenshot gets once uploaded,
/// known before the upload and written in the report.
pub fn asset_id(transaction: Uuid, secure_session: Uuid) -> Uuid {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(transaction.as_bytes());
    key[16..].copy_from_slice(secure_session.as_bytes());
    Uuid::from_bytes(md5::compute(key).0)
}

/// GET the categories (requestAbuseCategoriesCoro).
pub async fn categories(http: &reqwest::Client, url: &str) -> Option<Vec<(i32, String)>> {
    let response = http
        .get(url)
        .header("Accept", "application/llsd+xml")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .ok()?;
    let reply = reply(response).await.ok()?;
    parse_categories(&reply)
}

/// sendReportViaCaps: with SendUserReportWithScreenshot, the report is the
/// body of an upload whose file is the screenshot (as for any asset: ask
/// for an uploader, post the file, three tries); otherwise the report alone
/// to SendUserReport.
pub async fn send(
    http: &reqwest::Client,
    plain: Option<&str>,
    with_screenshot: Option<&str>,
    body: Llsd,
    screenshot: Option<Vec<u8>>,
) -> Result<(), String> {
    let failed = || aurora_i18n::tr!("report-abuse-send-failed").to_owned();
    if let Some(url) = with_screenshot {
        for attempt in 0..3 {
            let response = post_llsd(http, url, &body).await.map_err(|_| failed())?;
            let answer = reply(response).await.map_err(|_| failed())?;
            if answer.has("error") {
                return Err(failed());
            }
            let uploader = answer["uploader"].as_str();
            // "No upload url provided. Nothing uploaded, responding with
            // previous result."
            let (false, Some(data)) = (uploader.is_empty(), screenshot.as_ref()) else {
                return Ok(());
            };
            let response = http
                .post(uploader)
                .header("Content-Type", "application/octet-stream")
                .timeout(UPLOAD_TIMEOUT)
                .body(data.clone())
                .send()
                .await;
            if let Ok(response) = response
                && response.status().is_success()
                && let Ok(done) = reply(response).await
                && done["state"].as_str() == "complete"
            {
                return Ok(());
            }
            if attempt < 2 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        return Err(failed());
    }
    let url = plain.ok_or_else(failed)?;
    let response = post_llsd(http, url, &body).await.map_err(|_| failed())?;
    if response.status().is_success() { Ok(()) } else { Err(failed()) }
}

async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd) -> reqwest::Result<reqwest::Response> {
    http.post(url)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(body))
        .timeout(UPLOAD_TIMEOUT)
        .send()
        .await
}

/// An LLSD answer of at most 64 KiB; capability URLs never reach the logs.
async fn reply(mut response: reqwest::Response) -> Result<Llsd, ()> {
    if !response.status().is_success() {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(Llsd::new_map());
    }
    aurora_llsd::from_xml(&bytes).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_has_the_fields_of_gather_report() {
        let report = AbuseReport {
            category: 45,
            position: Vec3::new(128.5, 64.0, 22.25),
            object_id: Uuid::from_u128(1),
            abuser_id: Uuid::from_u128(2),
            summary: " |Plaza| (secondlife://Plaza/128/64/22) [Spam]  {Nova}  \"spam\"".into(),
            details: "V1.0\n\nAbuser name: Nova \n".into(),
            version_string: "1.0 Win CPU GPU driver".into(),
            screenshot: None,
        };
        let body = report.body(Uuid::from_u128(3));
        assert_eq!(body["report-type"].as_i32(), 2);
        assert_eq!(body["category"].as_i32(), 45);
        assert_eq!(body["position"].as_vec3(), [128.5, 64.0, 22.25]);
        assert_eq!(body["check-flags"].as_i32(), 0);
        assert_eq!(body["screenshot-id"].as_uuid(), Uuid::from_u128(3));
        assert_eq!(body["object-id"].as_uuid(), Uuid::from_u128(1));
        assert_eq!(body["abuser-id"].as_uuid(), Uuid::from_u128(2));
        assert_eq!(body["abuse-region-name"].as_str(), "");
        assert_eq!(body["abuse-region-id"].as_uuid(), Uuid::nil());
        assert_eq!(body["summary"].as_str(), report.summary);
        assert_eq!(body["details"].as_str(), report.details);
        assert_eq!(body["version-string"].as_str(), report.version_string);
        // nothing else
        assert_eq!(body.as_map().map(|m| m.len()), Some(12));
    }

    #[test]
    fn categories_follow_the_server_and_its_language() {
        assert_eq!(categories_url("https://cap", "fr"), "https://cap?lc=fr");
        assert_eq!(categories_url("https://cap", "default"), "https://cap");
        let reply = llsd_map! {
            "categories" => vec![
                llsd_map! { "category" => 31, "description_localized" => "Age > Age play" },
                llsd_map! { "category" => 0, "description_localized" => "Select category" },
                llsd_map! { "category" => 67, "description_localized" => "Skill Gaming" },
            ],
        };
        assert_eq!(
            parse_categories(&reply),
            Some(vec![(31, "Age > Age play".into()), (67, "Skill Gaming".into())])
        );
        assert_eq!(parse_categories(&Llsd::new_map()), None);
    }

    #[test]
    fn the_screenshot_id_is_the_transaction_asset_id() {
        let transaction = Uuid::from_u128(0x1234);
        let session = Uuid::from_u128(0x5678);
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(transaction.as_bytes());
        key[16..].copy_from_slice(session.as_bytes());
        assert_eq!(asset_id(transaction, session), Uuid::from_bytes(md5::compute(key).0));
        assert_ne!(asset_id(transaction, session), asset_id(transaction, Uuid::nil()));
    }

    #[tokio::test]
    async fn the_report_is_the_body_of_the_capability_request() {
        let report = AbuseReport {
            category: 45,
            summary: "spam".into(),
            ..Default::default()
        };
        let body = report.body(Uuid::from_u128(3));
        // SendUserReport: the report alone
        let (url, task) = crate::outfits::tests::server(vec![(200, Llsd::new_map())]).await;
        assert!(send(&reqwest::Client::new(), Some(&url), None, body.clone(), None).await.is_ok());
        let requests = task.await.expect("server");
        assert!(requests[0].0.starts_with("POST "));
        assert_eq!(requests[0].1, body);
        // SendUserReportWithScreenshot first; no uploader given: nothing to
        // upload, the report stands
        let (url, task) = crate::outfits::tests::server(vec![(200, Llsd::new_map())]).await;
        assert!(
            send(&reqwest::Client::new(), Some("unused"), Some(&url), body.clone(), Some(vec![1]))
                .await
                .is_ok()
        );
        assert_eq!(task.await.expect("server")[0].1, body);
        // a refusal is an error, without the capability in it
        let (url, task) = crate::outfits::tests::server(vec![(500, Llsd::new_map())]).await;
        let error = send(&reqwest::Client::new(), Some(&url), None, body, None)
            .await
            .expect_err("refused");
        assert!(!error.contains(&url));
        task.await.expect("server");
    }
}
