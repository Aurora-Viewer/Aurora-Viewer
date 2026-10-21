//! A photo sent by e-mail: the SendPostcard capability of the agent's
//! region.
//!
//! Port of LLPanelSnapshotPostcard::onSend / sendPostcard
//! (indra/newview/llpanelsnapshotpostcard.cpp), LLPostcardUploadInfo
//! (llpostcard.cpp) and LLViewerAssetUpload::AssetInventoryUploadCoproc
//! (llviewerassetupload.cpp), originally LGPL 2.1: the postcard's fields
//! are POSTed to the capability, which answers with an uploader; the JPEG
//! goes there, three tries at most, until its state is "complete".

use aurora_llsd::{Llsd, llsd_map};
use std::time::Duration;

/// LL_ASSET_UPLOAD_TIMEOUT_SEC.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// What the e-mail says.
#[derive(Debug, Clone, PartialEq)]
pub struct Postcard {
    /// The recipients, separated by commas.
    pub to: String,
    /// The sender's name as written.
    pub name: String,
    pub subject: String,
    pub message: String,
    /// Where the photo was taken, global metres (the e-mail's SLurl).
    pub pos_global: [f64; 3],
    /// The sender's address, given off Second Life only (OpenSim needs it).
    pub from: Option<String>,
}

/// generatePostBody.
pub fn body(p: &Postcard) -> Llsd {
    let mut body = llsd_map! {
        "pos-global" => Llsd::Array(p.pos_global.iter().map(|v| Llsd::Real(*v)).collect()),
        "to" => p.to.as_str(),
        "name" => p.name.as_str(),
        "subject" => p.subject.as_str(),
        "msg" => p.message.as_str(),
    };
    if let (Some(from), Llsd::Map(m)) = (&p.from, &mut body) {
        m.insert("from".into(), Llsd::from(from.as_str()));
    }
    body
}

/// onSend's check of the recipients: one or more addresses separated by
/// commas (the panel's email_format).
pub fn valid_recipients(to: &str) -> bool {
    !to.trim().is_empty() && to.split(',').all(|a| valid_address(a.trim_start_matches([' ', '\t'])))
}

fn valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    let Some((host, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    !local.is_empty()
        && local.chars().all(|c| c.is_ascii_alphanumeric() || ".%+-_".contains(c))
        && !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
}

/// Why a postcard was not sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// 404 from the uploader: the photo is over the server's limit
    /// (CannotUploadSnapshotEmailTooBig).
    #[error("the photo is too big")]
    TooBig,
    #[error("{0}")]
    Failed(String),
}

/// Send the postcard and its JPEG.
pub async fn send(http: &reqwest::Client, url: &str, postcard: &Postcard, jpeg: Vec<u8>) -> Result<(), Error> {
    let body = aurora_llsd::to_xml(&body(postcard));
    let mut last = Error::Failed("no answer".into());
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // stage 1: the uploader
        let answer = http
            .post(url)
            .header("Content-Type", "application/llsd+xml")
            .header("Accept", "application/llsd+xml")
            .timeout(UPLOAD_TIMEOUT)
            .body(body.clone())
            .send()
            .await
            .map_err(|e| Error::Failed(e.to_string()))?;
        let answer = llsd_reply(answer).await?;
        if answer.has("error") {
            return Err(Error::Failed(answer["error"]["message"].to_string_value()));
        }
        let uploader = answer["uploader"].as_str();
        if uploader.is_empty() {
            return Err(Error::Failed("no uploader".into()));
        }
        // stage 2: the photo
        let sent = http
            .post(uploader)
            .header("Content-Type", "application/octet-stream")
            .header("Accept", "application/llsd+xml")
            .timeout(UPLOAD_TIMEOUT)
            .body(jpeg.clone())
            .send()
            .await;
        match sent {
            Ok(r) if r.status().as_u16() == 404 => last = Error::TooBig,
            Ok(r) => match llsd_reply(r).await {
                Ok(done) if done["state"].as_str() == "complete" => return Ok(()),
                Ok(done) => last = Error::Failed(format!("state {}", done["state"].to_string_value())),
                Err(e) => last = e,
            },
            Err(e) => last = Error::Failed(e.to_string()),
        }
        log::warn!("postcard: upload attempt {} failed ({last})", attempt + 1);
    }
    Err(last)
}

async fn llsd_reply(r: reqwest::Response) -> Result<Llsd, Error> {
    let status = r.status();
    if status.as_u16() == 404 {
        return Err(Error::TooBig);
    }
    if !status.is_success() {
        return Err(Error::Failed(format!("status {}", status.as_u16())));
    }
    let bytes = r.bytes().await.map_err(|e| Error::Failed(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(Llsd::Undef);
    }
    aurora_llsd::from_xml(&bytes).map_err(|e| Error::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(from: Option<&str>) -> Postcard {
        Postcard {
            to: "a@example.org".into(),
            name: "Nova Demo".into(),
            subject: "Postcard".into(),
            message: "Look!".into(),
            pos_global: [256_128.0, 256_640.5, 22.25],
            from: from.map(str::to_owned),
        }
    }

    #[test]
    fn the_body_carries_the_legacy_fields() {
        let b = aurora_llsd::from_xml(&aurora_llsd::to_xml(&body(&card(None)))).unwrap();
        assert_eq!(b["to"].as_str(), "a@example.org");
        assert_eq!(b["name"].as_str(), "Nova Demo");
        assert_eq!(b["subject"].as_str(), "Postcard");
        assert_eq!(b["msg"].as_str(), "Look!");
        let pos: Vec<f64> = b["pos-global"].as_array().iter().map(Llsd::as_f64).collect();
        assert_eq!(pos, vec![256_128.0, 256_640.5, 22.25]);
        assert!(!b.has("from"), "Second Life knows the sender");
        let b = body(&card(Some("me@example.org")));
        assert_eq!(b["from"].as_str(), "me@example.org");
    }

    #[test]
    fn recipients_are_addresses_separated_by_commas() {
        assert!(valid_recipients("a@example.org"));
        assert!(valid_recipients("first.last+tag@mail.example.co.uk"));
        assert!(valid_recipients("a@example.org, b@example.net,\tc@example.com"));
        assert!(!valid_recipients(""));
        assert!(!valid_recipients("a@example"));
        assert!(!valid_recipients("a@example.o"));
        assert!(!valid_recipients("@example.org"));
        assert!(!valid_recipients("a example@org.com"));
        assert!(!valid_recipients("a@example.org,"));
        assert!(!valid_recipients("a@example.org; b@example.org"));
    }
}
