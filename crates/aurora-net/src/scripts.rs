//! Script assets and uploads: LLPreviewLSL / LLLiveLSLEditor,
//! LLScriptAssetUpload::generatePostBody (Firestorm indra/newview,
//! originally LGPL 2.1). Source retrieval uses inventory permissions,
//! not ViewerAsset; uploads return the saved asset even when compilation fails.

use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

pub mod experiences;

#[derive(Debug, Clone, Copy)]
pub struct ScriptOptions {
    pub mono: bool,
    /// Wins over `mono` when not `Off`: the Luau VM keeps its target.
    pub luau: Luau,
    pub running: bool,
    pub experience: Uuid,
}

/// II_FLAGS_SUBTYPE_MASK (indra/llinventory/llinventorydefines.h): a
/// script item's subtype, its source language, sits in the low byte of
/// its flags.
const SUBTYPE_MASK: u32 = 0xff;
/// SST_LUA (ScriptSubtype_t, indra/llinventory/llinventorytype.h).
const SUBTYPE_LUA: u32 = 1;

/// A script item written in Lua (SLua), from its inventory flags, as
/// LLPreviewLSL::onLoadComplete and LLFloaterCompileQueue read
/// `getInventorySubType() == SST_LUA`.
pub fn lua_source(flags: u32) -> bool {
    flags & SUBTYPE_MASK == SUBTYPE_LUA
}

/// How a script uses the Luau VM, the compile targets beyond "lsl2" and
/// "mono" (LLLiveLSLEditor::processScriptRunningReply).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Luau {
    /// Not on Luau: "mono" or "lsl2".
    #[default]
    Off,
    /// LSL source on the Luau VM: "lsl-luau".
    Lsl,
    /// Lua source: "luau", its only target.
    Lua,
}

impl Luau {
    /// From a script item's flags: a Lua source keeps "luau" whatever its
    /// running reply says.
    pub fn from_flags(flags: u32) -> Self {
        if lua_source(flags) { Self::Lua } else { Self::Off }
    }

    /// The event queue's ScriptRunningReply: `Luau` is the Luau VM,
    /// `LuauLanguage` a Lua source.
    pub fn from_running(luau: bool, lua_language: bool) -> Self {
        match (luau, lua_language) {
            (true, true) => Self::Lua,
            (true, false) => Self::Lsl,
            _ => Self::Off,
        }
    }
}

/// LLScriptAssetUpload's "target".
pub fn compile_target(mono: bool, luau: Luau) -> &'static str {
    match luau {
        Luau::Lua => "luau",
        Luau::Lsl => "lsl-luau",
        Luau::Off if mono => "mono",
        Luau::Off => "lsl2",
    }
}

pub const MAX_SOURCE: usize = 1024 * 1024;
pub const DEFAULT_SOURCE: &str = "default\n{\n    state_entry()\n    {\n        llSay(0, \"Hello, Avatar!\");\n    }\n\n    touch_start(integer total_number)\n    {\n        llSay(0, \"Touched.\");\n    }\n}\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptTarget {
    /// Nil for an agent inventory item; otherwise the containing prim UUID.
    pub object: Uuid,
    pub item: Uuid,
}

#[derive(Debug, Clone)]
pub struct ScriptSaved {
    pub asset: Uuid,
    /// No compile error. Also true when the grid did not compile it.
    pub compiled: bool,
    /// The grid compiled it. OpenSim does not compile a script saved in
    /// the agent's inventory: its reply (LLSDAssetUploadComplete,
    /// UpdateItemAsset.cs) has no `compiled`, which is no failure.
    pub checked: bool,
    pub errors: Vec<String>,
}

/// A save's reply holds no compile failure: `compiled` is true, or absent
/// with no `errors` (a grid that did not compile the script, see
/// [`ScriptSaved::checked`]). A reply that lists errors is a failure even
/// without `compiled`.
pub fn compile_ok(ack: &Llsd) -> bool {
    match &ack["compiled"] {
        Llsd::Undef => ack["errors"].as_array().is_empty(),
        _ => ack["compiled"].as_bool(),
    }
}

pub fn upload_body(target: ScriptTarget, options: ScriptOptions, experience: Option<Uuid>) -> Llsd {
    let running = options.running;
    let mut body = llsd_map! { "item_id" => target.item, "target" => compile_target(options.mono, options.luau) };
    if !target.object.is_nil() {
        body.insert("task_id", target.object);
        body.insert("is_script_running", i32::from(running));
        body.insert("experience", experience.unwrap_or_default());
    }
    body
}

fn saved_reply(ack: &Llsd) -> Result<ScriptSaved, String> {
    if ack["state"].as_str() != "complete" || ack["new_asset"].as_uuid().is_nil() {
        return Err(aurora_i18n::tr!("script-save-unconfirmed").into());
    }
    let errors = ack["errors"].as_array().iter().map(Llsd::to_string_value).collect();
    Ok(ScriptSaved {
        asset: ack["new_asset"].as_uuid(),
        compiled: compile_ok(ack),
        checked: !matches!(ack["compiled"], Llsd::Undef),
        errors,
    })
}

pub async fn save(
    http: &reqwest::Client,
    cap: &str,
    target: ScriptTarget,
    options: ScriptOptions,
    source: String,
) -> Result<ScriptSaved, String> {
    if source.len() > MAX_SOURCE {
        return Err(aurora_i18n::tr!("script-source-too-large").into());
    }
    // Cover all requests, including a server that never finishes its response.
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        // LLLiveLSLEditor uploads the association loaded / selected by the
        // editor. No metadata lookup at save time can block a plain script.
        let answer = crate::caps::post_llsd(http, cap, &upload_body(target, options, Some(options.experience)))
            .await
            .map_err(|_| aurora_i18n::tr!("script-save-failed").to_owned())?;
        if answer["state"].as_str() != "upload" || answer["uploader"].as_str().is_empty() {
            return Err(aurora_i18n::tr!("script-save-unconfirmed").into());
        }
        let mut response = http
            .post(answer["uploader"].as_str())
            .header("Content-Type", "application/octet-stream")
            .body(source.into_bytes())
            .send()
            .await
            .map_err(|_| aurora_i18n::tr!("script-save-failed").to_owned())?;
        if !response.status().is_success() {
            return Err(aurora_i18n::tr!("script-save-failed").into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| aurora_i18n::tr!("script-save-failed").to_owned())?
        {
            if bytes.len() + chunk.len() > MAX_SOURCE {
                return Err(aurora_i18n::tr!("script-save-unconfirmed").into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let ack = aurora_llsd::from_xml(&bytes).map_err(|_| aurora_i18n::tr!("script-save-unconfirmed").to_owned())?;
        saved_reply(&ack)
    })
    .await
    .map_err(|_| aurora_i18n::tr!("script-timeout").to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn saving_posts_llsd_then_the_exact_utf8_source_and_returns_server_errors() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("test listener");
        let url = format!("http://{}", listener.local_addr().expect("test address"));
        let asset = Uuid::from_u128(44);
        let experience = Uuid::from_u128(45);
        let replies = [
            llsd_map! { "state" => "upload", "uploader" => format!("{url}/source") },
            llsd_map! { "state" => "complete", "new_asset" => asset, "compiled" => false, "errors" => Llsd::Array(vec!["(2, 3) ERROR".into()]) },
        ];
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.expect("upload request");
                let mut data = Vec::new();
                let (start, len) = loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.expect("request bytes");
                    assert!(n > 0, "request truncated");
                    data.extend_from_slice(&buf[..n]);
                    if let Some(pos) = data.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..pos]);
                        let len = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .expect("content length");
                        if data.len() >= pos + 4 + len {
                            break (pos + 4, len);
                        }
                    }
                };
                requests.push((
                    String::from_utf8_lossy(&data[..start]).to_string(),
                    data[start..start + len].to_vec(),
                ));
                let body = aurora_llsd::to_xml(&reply);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.expect("response header");
                socket.write_all(&body).await.expect("response body");
                crate::test_http::finish(socket);
            }
            requests
        });
        let target = ScriptTarget {
            object: Uuid::from_u128(20),
            item: Uuid::from_u128(21),
        };
        let source = "default\n{ // éèあ\n    state_entry() { llSay(0, \"Bonjour !\"); }\n}\n";
        let options = ScriptOptions {
            mono: false,
            luau: Luau::Off,
            running: false,
            experience,
        };
        let result = save(&reqwest::Client::new(), &url, target, options, source.into())
            .await
            .expect("saved source");
        assert_eq!(result.asset, asset);
        assert!(!result.compiled);
        assert_eq!(result.errors, ["(2, 3) ERROR"]);
        let requests = server.await.expect("server");
        assert_eq!(requests.len(), 2);
        assert_eq!(
            aurora_llsd::from_xml(&requests[0].1).expect("LLSD"),
            upload_body(target, options, Some(experience))
        );
        assert!(requests[1].0.to_lowercase().contains("content-type: application/octet-stream"));
        assert_eq!(requests[1].1, source.as_bytes());
    }

    #[test]
    fn upload_fields_match_firestorm_for_inventory_and_tasks() {
        let target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(12),
        };
        let options = |mono, luau, running| ScriptOptions {
            mono,
            luau,
            running,
            experience: Uuid::nil(),
        };
        let body = upload_body(target, options(true, Luau::Off, false), None);
        assert_eq!(body["item_id"].as_uuid(), target.item);
        assert_eq!(body["target"].as_str(), "mono");
        assert!(body.as_map().expect("upload map").get("task_id").is_none());
        let task = ScriptTarget {
            object: Uuid::from_u128(13),
            ..target
        };
        let body = upload_body(task, options(false, Luau::Off, true), None);
        assert_eq!(body["task_id"].as_uuid(), task.object);
        assert_eq!(body["target"].as_str(), "lsl2");
        assert_eq!(body["is_script_running"].as_i32(), 1);
        assert_eq!(body["experience"].as_uuid(), Uuid::nil());
        // The Luau VM keeps its target whatever the Mono box says.
        for mono in [false, true] {
            assert_eq!(upload_body(task, options(mono, Luau::Lua, true), None)["target"].as_str(), "luau");
            assert_eq!(
                upload_body(task, options(mono, Luau::Lsl, true), None)["target"].as_str(),
                "lsl-luau"
            );
        }
    }

    #[test]
    fn a_lua_source_is_read_from_the_item_subtype_and_the_running_reply() {
        assert!(!lua_source(0));
        assert!(lua_source(1));
        // Only the low byte is the subtype: other flags bits do not count.
        assert!(lua_source(0x0100_0001));
        assert!(!lua_source(0x0100_0000));
        assert!(!lua_source(2));
        assert_eq!(Luau::from_flags(1), Luau::Lua);
        assert_eq!(Luau::from_flags(0), Luau::Off);
        assert_eq!(Luau::from_running(true, true), Luau::Lua);
        assert_eq!(Luau::from_running(true, false), Luau::Lsl);
        assert_eq!(Luau::from_running(false, false), Luau::Off);
        assert_eq!(Luau::from_running(false, true), Luau::Off);
    }

    #[test]
    fn compile_failure_still_returns_the_saved_source_asset_and_diagnostics() {
        let asset = Uuid::from_u128(42);
        let result = saved_reply(&llsd_map! {
            "state" => "complete", "new_asset" => asset, "compiled" => false,
            "errors" => Llsd::Array(vec!["(4, 8) : ERROR : Syntax error".into()])
        })
        .expect("source saved");
        assert_eq!(result.asset, asset);
        assert!(!result.compiled);
        assert_eq!(result.errors, ["(4, 8) : ERROR : Syntax error"]);
        assert!(saved_reply(&llsd_map! { "state" => "complete", "compiled" => true }).is_err());
        assert!(result.checked);
    }

    #[test]
    fn an_inventory_save_the_grid_did_not_compile_is_no_failure() {
        // OpenSim's reply to UpdateScriptAgent (LLSDAssetUploadComplete):
        // no `compiled` and no `errors`
        let asset = Uuid::from_u128(43);
        let ack = llsd_map! { "state" => "complete", "new_asset" => asset, "new_inventory_item" => Uuid::from_u128(44) };
        let result = saved_reply(&ack).expect("source saved");
        assert_eq!(result.asset, asset);
        assert!(result.compiled && !result.checked && result.errors.is_empty());
        assert!(compile_ok(&ack));
        assert!(!compile_ok(&llsd_map! { "compiled" => false }));
        assert!(compile_ok(&llsd_map! { "compiled" => true }));
        // errors without `compiled` stay a failure
        assert!(!compile_ok(
            &llsd_map! { "errors" => Llsd::Array(vec!["(4, 8) : ERROR : Syntax error".into()]) }
        ));
    }
}
