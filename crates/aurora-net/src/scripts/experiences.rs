//! LLLiveLSLEditor::requestExperiences and LLExperienceCache::fetchAssociatedExperienceCoro /
//! requestExperiences (Firestorm indra/newview and indra/llmessage, originally LGPL 2.1).
//! Association and creator permissions are independent: a failed list never clears an association.

use super::{MAX_SOURCE, ScriptTarget};
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ScriptExperience {
    pub id: Uuid,
    pub name: String,
    pub selectable: bool,
}

#[derive(Debug, Clone)]
pub struct ScriptExperiences {
    pub associated: Result<Uuid, String>,
    pub available: Result<Vec<ScriptExperience>, String>,
}

fn metadata_body(target: ScriptTarget) -> Llsd {
    llsd_map! { "object-id" => target.object, "item-id" => target.item, "fields" => Llsd::Array(vec!["experience".into()]) }
}

fn associated(reply: &Llsd) -> Result<Uuid, String> {
    match &reply["experience"] {
        // Firestorm's setAssociatedExperience also defaults to nil when the
        // simulator omits the field for an unassociated script.
        Llsd::Undef => Ok(Uuid::nil()),
        Llsd::Uuid(id) => Ok(*id),
        Llsd::String(s) if s.is_empty() => Ok(Uuid::nil()),
        Llsd::String(s) => Uuid::parse_str(s).map_err(|_| aurora_i18n::tr!("script-metadata-failed").into()),
        _ => Err(aurora_i18n::tr!("script-metadata-failed").into()),
    }
}

async fn request(request: reqwest::RequestBuilder) -> Result<Llsd, ()> {
    tokio::time::timeout(std::time::Duration::from_secs(15), response(request))
        .await
        .map_err(|_| ())?
}

async fn response(request: reqwest::RequestBuilder) -> Result<Llsd, ()> {
    let mut response = request.header("Accept", "application/llsd+xml").send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > MAX_SOURCE {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    aurora_llsd::from_xml(&bytes).map_err(|_| ())
}

pub async fn load(
    http: &reqwest::Client,
    target: ScriptTarget,
    metadata: Option<&str>,
    creators: Option<&str>,
    info: Option<&str>,
) -> ScriptExperiences {
    let association = async {
        let cap = metadata.ok_or_else(|| aurora_i18n::tr!("script-metadata-failed").to_owned())?;
        let reply = request(
            http.post(cap)
                .header("Content-Type", "application/llsd+xml")
                .body(aurora_llsd::to_xml(&metadata_body(target))),
        )
        .await
        .map_err(|_| aurora_i18n::tr!("script-metadata-failed").to_owned())?;
        associated(&reply)
    };
    let creator_list = async {
        let cap = creators.ok_or(())?;
        let reply = request(http.get(cap)).await?;
        if !matches!(reply["experience_ids"], Llsd::Array(_)) {
            return Err(());
        }
        let mut ids = std::collections::BTreeSet::new();
        for value in reply["experience_ids"].as_array() {
            let id = match value {
                Llsd::Uuid(id) => *id,
                Llsd::String(s) => Uuid::parse_str(s).map_err(|_| ())?,
                _ => return Err(()),
            };
            if !id.is_nil() {
                ids.insert(id);
            }
        }
        Ok(ids)
    };
    let (associated, creators) = tokio::join!(association, creator_list);
    let available = async {
        let creators = creators?;
        let mut choices: Vec<_> = creators
            .iter()
            .map(|id| ScriptExperience {
                id: *id,
                name: id.to_string(),
                selectable: true,
            })
            .collect();
        if let Ok(id) = associated.as_ref()
            && !id.is_nil()
            && !creators.contains(id)
        {
            choices.push(ScriptExperience {
                id: *id,
                name: id.to_string(),
                selectable: false,
            });
        }
        // Bound URI size and batch names as LLExperienceCache does. UUIDs
        // remain valid choices if a name lookup is temporarily unavailable.
        let names = async {
            if let Some(cap) = info {
                let ids: Vec<Uuid> = choices.iter().map(|c| c.id).collect();
                for (url, _) in crate::experience::info_urls(cap, &ids) {
                    if let Ok(reply) = request(http.get(url)).await {
                        for value in reply["experience_keys"].as_array() {
                            if let Some(choice) = choices.iter_mut().find(|e| e.id == value["public_id"].as_uuid()) {
                                let name = value["name"].as_str();
                                if !name.is_empty() {
                                    choice.name = name.to_owned();
                                }
                            }
                        }
                    }
                }
            }
        };
        // A name timeout keeps both the known association and permitted
        // UUID choices. It must not turn a successful metadata read into an error.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(15), names).await;
        choices.sort_by_cached_key(|choice| (choice.name.to_lowercase(), choice.id));
        Ok(choices)
    }
    .await
    .map_err(|()| aurora_i18n::tr!("script-experiences-unavailable").to_owned());
    ScriptExperiences { associated, available }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn opening_and_saving_with_no_experience_or_an_inaccessible_association() {
        for associated_id in [Uuid::nil(), Uuid::from_u128(42)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
            let base = format!("http://{}", listener.local_addr().expect("address"));
            let server_base = base.clone();
            let creator = Uuid::from_u128(45);
            let server = tokio::spawn(async move {
                let mut upload_count = 0;
                for _ in 0..5 {
                    let (mut socket, _) = listener.accept().await.expect("request");
                    let mut bytes = Vec::new();
                    let (headers, body) = loop {
                        let mut buffer = [0; 4096];
                        let len = socket.read(&mut buffer).await.expect("read");
                        assert!(len > 0);
                        bytes.extend_from_slice(&buffer[..len]);
                        if let Some(start) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&bytes[..start]).into_owned();
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    line.split_once(':')
                                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if bytes.len() >= start + 4 + length {
                                break (headers, bytes[start + 4..start + 4 + length].to_vec());
                            }
                        }
                    };
                    let reply = if headers.starts_with("POST /metadata ") {
                        let query = aurora_llsd::from_xml(&body).expect("query");
                        assert_eq!(query["object-id"].as_uuid(), Uuid::from_u128(20));
                        assert_eq!(query["item-id"].as_uuid(), Uuid::from_u128(21));
                        assert_eq!(query["fields"].at(0).as_str(), "experience");
                        if associated_id.is_nil() {
                            llsd_map! {}
                        } else {
                            llsd_map! { "experience" => associated_id }
                        }
                    } else if headers.starts_with("GET /creators ") {
                        llsd_map! { "experience_ids" => Llsd::Array(vec![creator.into()]) }
                    } else if headers.starts_with("GET /info/id/?") {
                        assert!(headers.contains(&format!("public_id={creator}")));
                        if !associated_id.is_nil() {
                            assert!(headers.contains(&format!("public_id={associated_id}")));
                        }
                        llsd_map! { "experience_keys" => Llsd::Array(vec![
                            llsd_map! { "public_id" => creator, "name" => "Creator experience" },
                            llsd_map! { "public_id" => associated_id, "name" => "Existing association" },
                        ]) }
                    } else if headers.starts_with("POST /upload ") {
                        upload_count += 1;
                        let upload = aurora_llsd::from_xml(&body).expect("upload");
                        assert_eq!(upload["experience"].as_uuid(), associated_id);
                        llsd_map! { "state" => "upload", "uploader" => format!("{server_base}/source") }
                    } else {
                        assert!(headers.starts_with("POST /source "));
                        assert_eq!(body, b"default {}\n");
                        llsd_map! { "state" => "complete", "new_asset" => Uuid::from_u128(99), "compiled" => true }
                    };
                    let body = aurora_llsd::to_xml(&reply);
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(header.as_bytes()).await.expect("header");
                    socket.write_all(&body).await.expect("body");
                }
                assert_eq!(upload_count, 1);
            });
            let target = ScriptTarget {
                object: Uuid::from_u128(20),
                item: Uuid::from_u128(21),
            };
            let http = reqwest::Client::new();
            let result = load(
                &http,
                target,
                Some(&format!("{base}/metadata")),
                Some(&format!("{base}/creators")),
                Some(&format!("{base}/info")),
            )
            .await;
            assert_eq!(result.associated, Ok(associated_id));
            let choices = result.available.expect("creator list");
            assert!(
                choices
                    .iter()
                    .any(|e| e.id == creator && e.name == "Creator experience" && e.selectable)
            );
            if !associated_id.is_nil() {
                assert!(choices.iter().any(|e| e.id == associated_id && !e.selectable));
            }
            let options = super::super::ScriptOptions {
                mono: true,
                running: true,
                experience: associated_id,
            };
            assert!(
                super::super::save(&http, &format!("{base}/upload"), target, options, "default {}\n".into())
                    .await
                    .expect("save")
                    .compiled
            );
            tokio::time::timeout(std::time::Duration::from_secs(5), server)
                .await
                .expect("server finished")
                .expect("server");
        }
    }

    #[test]
    fn absent_undefined_empty_and_nil_metadata_mean_no_experience() {
        for reply in [
            llsd_map! {},
            llsd_map! { "experience" => Llsd::Undef },
            llsd_map! { "experience" => "" },
            llsd_map! { "experience" => Uuid::nil() },
        ] {
            assert_eq!(associated(&reply), Ok(Uuid::nil()));
        }
        let id = Uuid::from_u128(45);
        assert_eq!(associated(&llsd_map! { "experience" => id }), Ok(id));
        assert_eq!(associated(&llsd_map! { "experience" => id.to_string() }), Ok(id));
        assert!(associated(&llsd_map! { "experience" => "invalid" }).is_err());
        assert!(associated(&llsd_map! { "experience" => true }).is_err());
    }
}
