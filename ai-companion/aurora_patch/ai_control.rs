//! Opt-in localhost-only AI control bridge for an external local agent.
//! No account secrets, arbitrary HTTP requests, payments, inventory changes,
//! file operations or raw key injection are exposed.
use super::*;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

const ADDR: &str = "127.0.0.1:57833";
const MAX_REQUEST_BYTES: u64 = 8192;

struct Pending {
    body: serde_json::Value,
    respond: mpsc::Sender<serde_json::Value>,
}

pub(super) struct Bridge {
    incoming: Receiver<Pending>,
    movement: Option<(MoveInput, Instant, Instant)>,
    last_chat: Option<Instant>,
    read_chat: bool,
    allow_chat: bool,
}

impl Bridge {
    /// Disabled unless the operator explicitly starts Aurora with a secret.
    pub(super) fn start() -> Option<Self> {
        let token = std::env::var("AURORA_AI_TOKEN").ok()?;
        if token.len() < 32 {
            log::warn!("AI bridge disabled: token must have at least 32 characters");
            return None;
        }
        let listener = match TcpListener::bind(ADDR) {
            Ok(listener) => listener,
            Err(error) => {
                log::warn!("AI bridge disabled: cannot bind {ADDR}: {error}");
                return None;
            }
        };
        let (tx, rx) = mpsc::sync_channel::<Pending>(16);
        let _ = std::thread::Builder::new()
            .name("aurora-ai-local".to_owned())
            .spawn(move || {
                for connection in listener.incoming() {
                    let Ok(mut stream) = connection else { continue };
                    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(3))) {
                        log::debug!("AI bridge stream: {error}");
                        continue;
                    }
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    let result = handle_socket(&mut stream, &token, &tx);
                    let reply = result.unwrap_or_else(|error| {
                        serde_json::json!({"ok":false,"error":error})
                    });
                    if let Ok(encoded) = serde_json::to_vec(&reply) {
                        let _ = stream.write_all(&encoded);
                        let _ = stream.write_all(b"\n");
                    }
                }
            });
        log::info!("AI bridge enabled on {ADDR} (loopback only)");
        Some(Self {
            incoming: rx,
            movement: None,
            last_chat: None,
            read_chat: std::env::var("AURORA_AI_CHAT_READ").as_deref() == Ok("1"),
            allow_chat: std::env::var("AURORA_AI_ALLOW_CHAT").as_deref() == Ok("1"),
        })
    }

    fn next(&self) -> Option<Pending> {
        self.incoming.try_recv().ok()
    }

    pub(super) fn apply_move_input(&self, input: &mut MoveInput) {
        // A human pressing a movement control always has priority.
        let manual = input.forward || input.back || input.turn_left || input.turn_right
            || input.strafe_left || input.strafe_right || input.up || input.down
            || input.joystick_yaw != 0.0 || input.joystick_pitch != 0.0;
        if manual { return; }
        if let Some((requested, started, until)) = self.movement
            && Instant::now() < until
        {
            *input = requested;
            input.turn_held = started.elapsed().as_secs_f32();
        }
    }
}

fn handle_socket(stream: &mut TcpStream, token: &str, tx: &SyncSender<Pending>) -> Result<serde_json::Value, &'static str> {
    let mut reader = std::io::BufReader::new(&mut *stream);
    let mut data = Vec::new();
    let count = reader.take(MAX_REQUEST_BYTES + 1).read_until(b'\n', &mut data)
        .map_err(|_| "request read failed")?;
    if count == 0 || count as u64 > MAX_REQUEST_BYTES || !data.ends_with(b"\n") {
        return Err("request too large or incomplete");
    }
    let value: serde_json::Value = serde_json::from_slice(&data).map_err(|_| "invalid JSON")?;
    if value.get("token").and_then(serde_json::Value::as_str) != Some(token) {
        return Err("unauthorized");
    }
    let (reply_tx, reply_rx) = mpsc::channel();
    tx.try_send(Pending { body: value, respond: reply_tx }).map_err(|_| "viewer busy")?;
    reply_rx.recv_timeout(Duration::from_secs(4)).map_err(|_| "viewer not responding")
}

impl App {
    pub(super) fn poll_ai_bridge(&mut self) {
        // Drain at most four per rendered frame, never blocking the render thread.
        for _ in 0..4 {
            let Some(req) = self.ai_bridge.as_ref().and_then(Bridge::next) else { break };
            let reply = self.ai_request(&req.body);
            let _ = req.respond.send(reply);
        }
    }

    fn ai_request(&mut self, request: &serde_json::Value) -> serde_json::Value {
        let op = request.get("op").and_then(serde_json::Value::as_str).unwrap_or("");
        if op == "status" {
            return serde_json::json!({"ok":true,"bridge":"aurora-ai-v0.2","logged_in":self.in_world() && self.world.main_region.is_some()});
        }
        if op == "stop" {
            if let Some(bridge) = self.ai_bridge.as_mut() { bridge.movement = None; }
            return serde_json::json!({"ok":true});
        }
        if !self.in_world() || self.world.main_region.is_none() || self.world.teleporting {
            return serde_json::json!({"ok":false,"error":"avatar not ready"});
        }
        match op {
            "observe" => {
                let p = self.world.agent.position;
                let avatars = self.world.objects.iter()
                    .filter(|(_, o)| o.is_avatar() && o.full_id != self.world.agent_id
                        && Some(o.key.region) == self.world.main_region)
                    .count();
                let read_chat = self.ai_bridge.as_ref().is_some_and(|bridge| bridge.read_chat);
                let mut chat = if read_chat {
                    self.world.chat.iter().rev()
                        .filter(|line| line.origin == crate::world::Origin::Live
                            && matches!(line.kind, crate::world::ChatKind::Local(_))
                            && line.source != self.world.agent_id)
                        .take(8)
                        .map(|line| serde_json::json!({
                            "from":line.from,
                            "text":line.text.chars().take(350).collect::<String>(),
                            "source":line.source.to_string(),
                            "time_ms":line.time.duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default().as_millis() as u64
                        }))
                        .collect::<Vec<_>>()
                } else { Vec::new() };
                chat.reverse();
                serde_json::json!({
                    "ok":true,"logged_in":true,
                    "agent_id":self.world.agent_id.to_string(),
                    "region":self.world.region_name(),
                    "position_region":[p.x,p.y,p.z],
                    "flying":self.world.agent.flying,
                    "seated":self.world.agent.is_sitting(),
                    "nearby_avatars":avatars,
                    "nearby_chat":chat,
                    "chat_read_enabled":read_chat
                })
            }
            "move" => {
                if self.world.agent.is_sitting() || self.autopilot.is_some() {
                    return serde_json::json!({"ok":false,"error":"seated or autopilot active"});
                }
                let direction = request.get("direction").and_then(serde_json::Value::as_str).unwrap_or("");
                let duration = request.get("duration_ms").and_then(serde_json::Value::as_u64).unwrap_or(400);
                if !(100..=1200).contains(&duration) {
                    return serde_json::json!({"ok":false,"error":"duration must be 100..1200 ms"});
                }
                let mut input = MoveInput::default();
                match direction {
                    "forward" => input.forward = true,
                    "back" => input.back = true,
                    "left" => input.turn_left = true,
                    "right" => input.turn_right = true,
                    "strafe_left" => input.strafe_left = true,
                    "strafe_right" => input.strafe_right = true,
                    _ => return serde_json::json!({"ok":false,"error":"unsupported movement"}),
                }
                if let Some(bridge) = self.ai_bridge.as_mut() {
                    let started = Instant::now();
                    bridge.movement = Some((input, started, started + Duration::from_millis(duration)));
                }
                serde_json::json!({"ok":true,"direction":direction,"duration_ms":duration})
            }
            "chat" => {
                let allow = self.ai_bridge.as_ref().is_some_and(|b| b.allow_chat);
                if !allow { return serde_json::json!({"ok":false,"error":"chat disabled"}); }
                let text = request.get("text").and_then(serde_json::Value::as_str).unwrap_or("").trim();
                if text.is_empty() || text.chars().count() > 180 || (text.contains('\n') || text.contains('\r')) {
                    return serde_json::json!({"ok":false,"error":"chat must be 1..180 characters on one line"});
                }
                if self.ai_bridge.as_ref().and_then(|b| b.last_chat).is_some_and(|at| at.elapsed() < Duration::from_secs(15)) {
                    return serde_json::json!({"ok":false,"error":"chat rate limit"});
                }
                self.send(NetCommand::Chat {
                    message:text.to_owned(), channel:0, chat_type:aurora_net::ChatType::Normal
                });
                if let Some(bridge) = self.ai_bridge.as_mut() { bridge.last_chat = Some(Instant::now()); }
                serde_json::json!({"ok":true})
            }
            _ => serde_json::json!({"ok":false,"error":"unknown operation"}),
        }
    }
}
