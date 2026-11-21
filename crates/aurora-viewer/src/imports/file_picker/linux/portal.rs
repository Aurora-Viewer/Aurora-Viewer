use super::super::Kind;
use super::{Cancellation, PathBuf, Request};
use futures_lite::{StreamExt, future::race};
use std::{collections::HashMap, time::Duration};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";

// Also type-checked by tests on Windows; only Linux opens a real portal.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) async fn select(request: &Request, cancellation: &Cancellation) -> anyhow::Result<Vec<PathBuf>> {
    let bus = Connection::session().await?;
    let chooser = Proxy::new(
        &bus,
        DESTINATION,
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    )
    .await?;
    // Subscribe BEFORE OpenFile: a fast response may precede its method reply.
    // Match the returned handle below, including portals that ignore our token.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DESTINATION)?
        .interface(REQUEST)?
        .member("Response")?
        .build();
    let mut responses = MessageStream::for_match_rule(rule, &bus, Some(64)).await?;
    if cancellation.cancelled() {
        return Ok(Vec::new());
    }
    let token = format!("aurora_import_{}", uuid::Uuid::new_v4().simple());
    let options = options(request, &token);
    // Do not drop this future on cancellation: first acquire the returned
    // handle, then Close it, even if Aurora closed while OpenFile was pending.
    let handle: OwnedObjectPath = chooser.call("OpenFile", &("", &request.title, options)).await?;
    let pending = Proxy::new(&bus, DESTINATION, handle.clone(), REQUEST).await?;
    let response = wait_for_response(
        async {
            while let Some(message) = responses.next().await {
                let message = message?;
                if message.header().path() == Some(&*handle) {
                    let (code, mut results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
                    if code != 0 {
                        return Ok(Vec::new());
                    }
                    let uris: Vec<String> = results
                        .remove("uris")
                        .ok_or_else(|| anyhow::anyhow!("file portal returned no URIs"))?
                        .try_into()?;
                    return uris
                        .iter()
                        .map(|uri| {
                            reqwest::Url::parse(uri)?
                                .to_file_path()
                                .map_err(|()| anyhow::anyhow!("file portal returned a non-local URI"))
                        })
                        .collect::<anyhow::Result<Vec<_>>>();
                }
            }
            Err(anyhow::anyhow!("file portal disconnected before its response"))
        },
        cancellation,
        || async { Ok(pending.call::<_, _, ()>("Close", &()).await?) },
    )
    .await;
    // Once a dialog was opened, an error must not unexpectedly open a second
    // picker via the fallback. Report it and finish this request instead.
    Ok(response.unwrap_or_else(|error| {
        log::warn!("import file portal response failed: {error}");
        Vec::new()
    }))
}

async fn wait_for_response<C: std::future::Future<Output = anyhow::Result<()>>>(
    response: impl std::future::Future<Output = anyhow::Result<Vec<PathBuf>>>,
    cancellation: &Cancellation,
    close: impl FnOnce() -> C,
) -> anyhow::Result<Vec<PathBuf>> {
    let response = race(response, async {
        while !cancellation.cancelled() {
            async_io::Timer::after(Duration::from_millis(50)).await;
        }
        Ok(Vec::new())
    })
    .await;
    if cancellation.cancelled() || response.is_err() {
        // Close ends the native interaction and emits NO Response signal.
        // Never wait for the response after this call.
        if let Err(error) = close().await {
            log::warn!("import file portal could not close its request: {error}");
        }
    }
    if cancellation.cancelled() { Ok(Vec::new()) } else { response }
}

fn options<'a>(request: &'a Request, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut options = HashMap::from([
        ("handle_token", Value::from(token)),
        ("modal", Value::from(false)),
        ("multiple", Value::from(request.kind == Kind::Files)),
        ("directory", Value::from(request.kind == Kind::Folder)),
    ]);
    if !request.extensions.is_empty() {
        let patterns: Vec<_> = request.extensions.iter().map(|ext| (0u32, format!("*.{ext}"))).collect();
        options.insert("filters", Value::from(vec![(&*request.label, patterns)]));
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;
    use std::cell::Cell;

    #[test]
    fn file_picker_portal_cancel_closes_without_waiting_for_a_response() {
        block_on(async {
            for before_open in [false, true] {
                let cancellation = Cancellation::default();
                if before_open {
                    cancellation.cancel();
                }
                let closed = Cell::new(0);
                let pending = async {
                    // Simulates closing Aurora while OpenFile is returning its
                    // handle; the portal will never emit Response after Close.
                    cancellation.cancel();
                    std::future::pending().await
                };
                let result = race(
                    wait_for_response(pending, &cancellation, || async {
                        closed.set(closed.get() + 1);
                        Ok(())
                    }),
                    async {
                        async_io::Timer::after(Duration::from_secs(2)).await;
                        panic!("cancellation must not wait for the Response signal");
                    },
                )
                .await
                .unwrap();
                assert!(result.is_empty());
                assert_eq!(closed.get(), 1);
            }
        });
    }

    #[test]
    fn file_picker_portal_completion_does_not_close_another_request() {
        block_on(async {
            for paths in [Vec::new(), vec![PathBuf::from("chosen.png")]] {
                let cancellation = Cancellation::default();
                let result = wait_for_response(async { Ok(paths.clone()) }, &cancellation, || async {
                    panic!("a finished picker needs no Close request")
                })
                .await
                .unwrap();
                assert_eq!(result, paths);
            }
            let closed = Cell::new(false);
            assert!(
                wait_for_response(async { Err(anyhow::anyhow!("disconnected")) }, &Cancellation::default(), || async {
                    closed.set(true);
                    Ok(())
                })
                .await
                .is_err()
            );
            assert!(closed.get());
        });
    }

    #[test]
    fn file_picker_portal_options_preserve_modes_filters_and_request_identity() {
        for kind in [Kind::File, Kind::Files, Kind::Folder] {
            let request = Request {
                kind,
                title: "Import".into(),
                label: "Textures".into(),
                extensions: if kind == Kind::Folder {
                    Vec::new()
                } else {
                    vec!["png".into(), "j2c".into()]
                },
            };
            let options = options(&request, "aurora_import_fixture");
            assert_eq!(options["handle_token"], Value::from("aurora_import_fixture"));
            assert_eq!(options["modal"], Value::from(false));
            assert_eq!(options["multiple"], Value::from(kind == Kind::Files));
            assert_eq!(options["directory"], Value::from(kind == Kind::Folder));
            if kind == Kind::Folder {
                assert!(!options.contains_key("filters"));
            } else {
                assert_eq!(
                    options["filters"],
                    Value::from(vec![("Textures", vec![(0u32, "*.png"), (0u32, "*.j2c")])])
                );
            }
        }
    }
}
