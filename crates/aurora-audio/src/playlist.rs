//! Resolve radio playlists before passing bytes to the audio decoder.
//!
//! LLAudioStreamManagerFMODSTUDIO delegates stream URLs to FMOD::createStream.
//! Aurora resolves PLS/M3U itself, and also the playlist exported by an AzuraCast
//! public player. This addition accepts only links on the player's HTTP(S)
//! origin, so page discovery cannot silently contact another site.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use url::Url;

use crate::http::{self, HttpResponse};

/// Why a station address did not lead to audio, so the viewer can say so.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlaylistError {
    /// The address, or a link a playlist gave, failed at the HTTP level
    /// (including a link to another origin, `HttpError::PlaylistOrigin`).
    #[error(transparent)]
    Http(#[from] http::HttpError),
    /// A web page that exports no radio playlist.
    #[error("{0}")]
    WebPage(&'static str),
    /// A playlist the radio decoder cannot follow: HLS, empty, a loop, too
    /// large, nested too deep or not UTF-8.
    #[error("{0}")]
    Playlist(&'static str),
    /// The document stopped arriving.
    #[error("radio playlist could not be read: {0}")]
    Read(std::io::Error),
}

const MAX_DOCUMENT_BYTES: u64 = 128 * 1024;
const MAX_DOCUMENTS: usize = 4;

#[derive(Clone, Copy)]
enum Document {
    Player,
    Pls,
    M3u,
}

fn document(response: &HttpResponse) -> Option<Document> {
    match response.content_type.as_deref().unwrap_or("") {
        "text/html" | "application/xhtml+xml" => return Some(Document::Player),
        "audio/x-scpls" | "application/pls+xml" => return Some(Document::Pls),
        "audio/x-mpegurl" | "audio/mpegurl" | "application/x-mpegurl" | "application/vnd.apple.mpegurl" => return Some(Document::M3u),
        "audio/mpeg" | "audio/aac" | "audio/aacp" | "audio/ogg" | "application/ogg" | "audio/wav" | "audio/x-wav" => return None,
        _ => {}
    }
    let path = response.url.path().to_ascii_lowercase();
    if path.ends_with(".pls") {
        Some(Document::Pls)
    } else if path.ends_with(".m3u") || path.ends_with(".m3u8") {
        Some(Document::M3u)
    } else {
        None
    }
}

pub(crate) fn open(url: &str, stop: &Arc<AtomicBool>) -> Result<HttpResponse, PlaylistError> {
    let mut response = http::open(url, stop)?;
    let origin = response.url.clone();
    let mut visited = vec![response.url.clone()];
    for _ in 0..MAX_DOCUMENTS {
        let Some(kind) = document(&response) else { return Ok(response) };
        let mut bytes = Vec::new();
        response
            .body
            .take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(PlaylistError::Read)?;
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(PlaylistError::Playlist("radio playlist or player page is too large"));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| PlaylistError::Playlist("radio playlist is not UTF-8"))?;
        let next = target(text, kind, &response.url)?;
        if visited.contains(&next) {
            return Err(PlaylistError::Playlist("radio playlist contains a loop"));
        }
        visited.push(next.clone());
        response = http::open_with_origin(next.as_str(), stop, Some(&origin))?;
    }
    if document(&response).is_none() {
        Ok(response)
    } else {
        Err(PlaylistError::Playlist("too many nested radio playlists"))
    }
}

fn target(text: &str, kind: Document, base: &Url) -> Result<Url, PlaylistError> {
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    let link = match kind {
        Document::Player => {
            // This is an inert JSON string in the public player's configuration,
            // not JavaScript to execute. JSON decoding handles escaped slashes.
            let value = text
                .split_once("\"downloadPlaylistUri\"")
                .and_then(|(_, rest)| rest.trim_start().strip_prefix(':'))
                .ok_or(PlaylistError::WebPage("web page does not export a radio playlist"))?;
            serde_json::Deserializer::from_str(value.trim_start())
                .into_iter::<String>()
                .next()
                .and_then(Result::ok)
                .ok_or(PlaylistError::WebPage("invalid player playlist link"))?
        }
        Document::Pls => pls_target(text)
            .ok_or(PlaylistError::Playlist("radio PLS playlist has no stream"))?
            .to_owned(),
        Document::M3u => m3u_target(text)?.to_owned(),
    };
    let mut next = base
        .join(link.trim())
        .map_err(|_| PlaylistError::Playlist("invalid radio playlist URL"))?;
    next.set_fragment(None);
    if !matches!(next.scheme(), "http" | "https")
        || next.origin() != base.origin()
        || !next.username().is_empty()
        || next.password().is_some()
    {
        return Err(http::HttpError::PlaylistOrigin.into());
    }
    Ok(next)
}

fn pls_target(text: &str) -> Option<&str> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if !key.get(..4)?.eq_ignore_ascii_case("file") {
                return None;
            }
            let index = key.get(4..)?.parse::<u32>().ok()?;
            let value = value.trim();
            (index > 0 && !value.is_empty()).then_some((index, value))
        })
        .min_by_key(|(index, _)| *index)
        .map(|(_, value)| value)
}

fn m3u_target(text: &str) -> Result<&str, PlaylistError> {
    if text.lines().any(|line| line.trim_start().starts_with("#EXT-X-")) {
        return Err(PlaylistError::Playlist("HLS playlists are not supported by the radio decoder"));
    }
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or(PlaylistError::Playlist("radio M3U playlist has no stream"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Shutdown, TcpListener};
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::decode::{DecodeStream, hint_for_path, tests::mp3_silence};
    use symphonia::core::io::ReadOnlySource;

    #[test]
    fn player_playlist_decodes_json_escapes_without_executing_scripts() {
        let base = Url::parse("https://radio.example/public/test").unwrap();
        let html = r#"<script>window.vueComponent('#player', {"downloadPlaylistUri":"\/public\/test\/playlist.pls"});</script>"#;
        assert_eq!(
            target(html, Document::Player, &base).unwrap().as_str(),
            "https://radio.example/public/test/playlist.pls"
        );
        for html in [
            "<html>Login required</html>",
            r#"{"downloadPlaylistUri":null}"#,
            r#"{"downloadPlaylistUri":"unterminated}"#,
        ] {
            assert!(target(html, Document::Player, &base).is_err());
        }
    }

    #[test]
    fn pls_uses_the_first_numbered_stream_and_preserves_query_strings() {
        let base = Url::parse("https://radio.example/public/test/playlist.pls").unwrap();
        let pls = "\u{feff}[playlist]\r\nFile10=/wrong\r\nfile1 = /listen/test/live?a=b=c\r\nTitle1=Station\r\nLength1=-1\r\nNumberOfEntries=2\r\n";
        assert_eq!(
            target(pls, Document::Pls, &base).unwrap().as_str(),
            "https://radio.example/listen/test/live?a=b=c"
        );
        for pls in ["[playlist]\nFile0=/invalid", "[playlist]\nFile1=", "[playlist]\nTitle1=No stream"] {
            assert!(target(pls, Document::Pls, &base).is_err());
        }
    }

    #[test]
    fn m3u_supports_relative_streams_but_rejects_hls_segments() {
        let base = Url::parse("https://radio.example/public/station.m3u").unwrap();
        let m3u = "\u{feff}#EXTM3U\r\n#EXTINF:-1,Station\r\n../listen/live\r\n";
        assert_eq!(
            target(m3u, Document::M3u, &base).unwrap().as_str(),
            "https://radio.example/listen/live"
        );
        for m3u in [
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=128000\nsegment.m3u8",
            "#EXTM3U\n#EXT-X-TARGETDURATION:5\nsegment.aac",
            "#EXTM3U\n",
        ] {
            assert!(target(m3u, Document::M3u, &base).is_err());
        }
    }

    #[test]
    fn playlist_discovery_never_expands_the_allowed_origin_or_scheme() {
        let base = Url::parse("https://radio.example/public/test").unwrap();
        for link in [
            "https://other.example/live",
            "//other.example/live",
            "https://radio.example:444/live",
            "http://radio.example/live",
            "file:///radio.mp3",
            "ftp://radio.example/live",
            "javascript:alert(1)",
            "https://name:password@radio.example/live",
        ] {
            assert!(target(&format!("File1={link}"), Document::Pls, &base).is_err(), "{link}");
        }
    }

    struct Route {
        path: &'static str,
        head: String,
        body: Vec<u8>,
    }

    fn serve(routes: Vec<Route>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            for route in routes {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "no request for {}", route.path);
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("test listener: {e}"),
                    }
                };
                // On macOS (BSD sockets) an accepted socket inherits the
                // listener's O_NONBLOCK, so the reads below would fail with
                // WouldBlock instead of waiting for the request.
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request.split_whitespace().nth(1), Some(route.path));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                write!(
                    socket,
                    "{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    route.head,
                    route.body.len()
                )
                .unwrap();
                let _ = socket.write_all(&route.body);
                let _ = socket.shutdown(Shutdown::Write);
                // Keep the socket alive until the client has consumed its body.
                let mut sink = Vec::new();
                let _ = reader.read_to_end(&mut sink);
            }
        });
        (base, server)
    }

    #[test]
    fn azuracast_player_and_pls_resolve_to_decodable_audio() {
        let (base, server) = serve(vec![
            Route {
                path: "/public/test",
                head: "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8".into(),
                body: br#"<script>window.vueComponent('#player', {"downloadPlaylistUri":"\/public\/test\/playlist.pls"});</script>"#
                    .to_vec(),
            },
            Route {
                path: "/public/test/playlist.pls",
                head: "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls".into(),
                body: b"[playlist]\nFile1=/listen/test/live\nTitle1=Station\nLength1=-1\nNumberOfEntries=1\nVersion=2".to_vec(),
            },
            Route {
                path: "/listen/test/live",
                head: "ICY 200 OK\r\nContent-Type: audio/mpeg".into(),
                body: mp3_silence(50),
            },
        ]);
        let stop = Arc::new(AtomicBool::new(false));
        let response = open(&format!("{base}/public/test"), &stop).unwrap();
        assert_eq!(response.url.path(), "/listen/test/live");
        assert_eq!(response.content_type.as_deref(), Some("audio/mpeg"));
        let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(response.body)), Some("mp3")).unwrap();
        assert!(!decoder.next_chunk().unwrap().unwrap().samples.is_empty());
        drop(decoder);
        server.join().unwrap();
    }

    #[test]
    fn redirected_playlists_resolve_relative_to_the_final_response_url() {
        let (base, server) = serve(vec![
            Route {
                path: "/radio.m3u",
                head: "HTTP/1.1 302 Found\r\nLocation: /station/list.m3u".into(),
                body: vec![],
            },
            Route {
                path: "/station/list.m3u",
                head: "HTTP/1.1 200 OK\r\nContent-Type: text/plain".into(),
                body: b"#EXTM3U\n../stream.mp3".to_vec(),
            },
            Route {
                path: "/stream.mp3",
                head: "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream".into(),
                body: mp3_silence(20),
            },
        ]);
        let stop = Arc::new(AtomicBool::new(false));
        let response = open(&format!("{base}/radio.m3u"), &stop).unwrap();
        let hint = hint_for_path(response.url.path());
        assert_eq!(hint, Some("mp3"));
        let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(response.body)), hint).unwrap();
        assert!(decoder.next_chunk().unwrap().is_some());
        drop(decoder);
        server.join().unwrap();
    }

    #[test]
    fn discovered_redirects_cannot_contact_a_second_origin() {
        let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
        forbidden.set_nonblocking(true).unwrap();
        let redirect_head = format!("HTTP/1.1 302 Found\r\nLocation: http://{}/stream", forbidden.local_addr().unwrap());
        let (base, server) = serve(vec![
            Route {
                path: "/list.pls",
                head: "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls".into(),
                body: b"[playlist]\nFile1=/redirect".to_vec(),
            },
            Route {
                path: "/redirect",
                head: redirect_head,
                body: vec![],
            },
        ]);
        let error = open(&format!("{base}/list.pls"), &Arc::new(AtomicBool::new(false))).err().unwrap();
        assert!(matches!(error, PlaylistError::Http(http::HttpError::PlaylistOrigin)), "{error}");
        assert!(error.to_string().contains("same HTTP(S) origin"), "{error}");
        assert_eq!(forbidden.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        server.join().unwrap();
    }

    #[test]
    fn playlist_loops_oversized_documents_and_cancellation_are_bounded() {
        for body in [b"[playlist]\nFile1=/list.pls".to_vec(), vec![b'x'; MAX_DOCUMENT_BYTES as usize + 1]] {
            let (base, server) = serve(vec![Route {
                path: "/list.pls",
                head: "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls".into(),
                body,
            }]);
            let error = open(&format!("{base}/list.pls"), &Arc::new(AtomicBool::new(false)))
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("loop") || error.contains("too large"), "{error}");
            server.join().unwrap();
        }
        let stop = Arc::new(AtomicBool::new(true));
        assert_eq!(open("http://127.0.0.1:1/list.pls", &stop).err().unwrap().to_string(), "stopped");
    }

    #[test]
    fn playlist_nesting_stops_after_the_document_budget() {
        let paths = ["/a.pls", "/b.pls", "/c.pls", "/d.pls", "/e.pls"];
        let routes = paths
            .iter()
            .enumerate()
            .map(|(i, path)| Route {
                path,
                head: "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls".into(),
                body: format!("[playlist]\nFile1={}", paths.get(i + 1).unwrap_or(&"/last.pls")).into_bytes(),
            })
            .collect();
        let (base, server) = serve(routes);
        let error = open(&format!("{base}/a.pls"), &Arc::new(AtomicBool::new(false)))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("too many nested"), "{error}");
        server.join().unwrap();
    }

    #[test]
    #[ignore = "contacts the public Lightman station; explicit manual network probe only"]
    fn lightman_live_probe() {
        use crate::icy::IcyReader;
        use parking_lot::Mutex;

        for url in [
            "https://login-na2.lightmanstreams.com/public/auto_dj_44",
            "https://login-na2.lightmanstreams.com/listen/auto_dj_44/live",
        ] {
            let stop = Arc::new(AtomicBool::new(false));
            let response = open(url, &stop).unwrap();
            eprintln!("{url} -> {} ({:?})", response.url, response.content_type);
            let title = Arc::new(Mutex::new(None));
            let reader: Box<dyn Read + Send + Sync> = match response.metaint {
                Some(interval) => Box::new(IcyReader::new(response.body, interval, title.clone())),
                None => response.body,
            };
            let mut decoder = DecodeStream::open(Box::new(ReadOnlySource::new(reader)), Some("mp3")).unwrap();
            let mut frames = 0;
            let mut peak = 0.0_f32;
            while frames < 44_100 {
                let chunk = decoder.next_chunk().unwrap().expect("live audio chunk");
                frames += chunk.samples.len() / chunk.channels;
                peak = chunk.samples.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
            }
            stop.store(true, Ordering::Relaxed);
            eprintln!("decoded {frames} frames, peak={peak}, title={:?}", *title.lock());
        }
    }
}
