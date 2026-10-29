//! The translation state of the conversations, without threads nor network:
//! jobs go out, results come back as effects for the app to apply.

use aurora_translate::{Direction, Done, Job, Lang, detect, detect_between, nothing_to_translate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// How long our message may wait for its translation before the line offers
/// to send the original.
pub const OUTGOING_TIMEOUT: Duration = Duration::from_secs(10);
/// The simulator's limit of an IM (bytes).
pub const MAX_IM_BYTES: usize = 1023;
/// whatlang's confidence, between my language and the partner's, that a
/// message is already in mine (a full sentence comes out at 1.0).
const MINE: f64 = 0.5;
/// whatlang's confidence among all languages for the suggestion: chat-length
/// text rarely goes above 0.7, two messages in a row make up for it.
const SUGGEST_CONFIDENCE: f64 = 0.5;
/// Shorter messages say too little about their language.
const SUGGEST_MIN_CHARS: usize = 15;

/// Preferences > Chat > Translation, and the language of each conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslateSettings {
    /// ISO 639-1 code; empty: the interface's language.
    pub my_language: String,
    pub model: String,
    /// Show our translated message for a last look before it leaves.
    pub preview: bool,
    /// The partner language of each translated conversation.
    pub sessions: HashMap<Uuid, String>,
    /// Conversations where "Not now" silenced the suggestion.
    pub dismissed: HashSet<Uuid>,
}

impl Default for TranslateSettings {
    fn default() -> Self {
        Self {
            my_language: String::new(),
            model: aurora_translate::DEFAULT_MODEL.into(),
            preview: false,
            sessions: HashMap::new(),
            dismissed: HashSet::new(),
        }
    }
}

/// A line of a conversation: its session and the time it was pushed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineKey {
    pub session: Uuid,
    pub time: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    Error,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineState {
    Pending,
    /// The line shows the translation; `original` is what was typed or
    /// received.
    Translated {
        original: String,
    },
    Failed {
        reason: FailReason,
    },
    /// Our translation is over the IM limit: not sent.
    TooLong {
        translation: String,
    },
}

/// What the app does with a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Send { to: Uuid, text: String },
    Replace { key: LineKey, text: String },
}

#[derive(Debug)]
struct Entry {
    state: LineState,
    /// The job in flight, if any.
    job: Option<u64>,
    direction: Direction,
    /// "/me " or "/me'" kept out of the translation.
    prefix: &'static str,
    /// The text without its prefix.
    body: String,
    from: &'static Lang,
    to: &'static Lang,
    /// When the model began our message (after its load and the jobs
    /// before it): the timeout counts from there.
    started: Option<Instant>,
    /// Our translated message, waiting for the earlier ones to leave.
    ready: Option<String>,
}

/// The lines being translated, and the order our messages leave in.
#[derive(Debug, Default)]
pub struct Translations {
    lines: HashMap<LineKey, Entry>,
    jobs: HashMap<u64, LineKey>,
    /// Our messages of each conversation, in typed order, until they leave.
    queues: HashMap<Uuid, VecDeque<LineKey>>,
    next: u64,
    /// Bumped at every change: the app rebuilds the interface's view then.
    version: u64,
}

/// What a line of a conversation shows about its translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineShow {
    /// Translating ("…" after the text).
    Pending,
    /// The line shows the translation; this was typed or received.
    Original(String),
    /// A received message the model could not translate.
    NotTranslated,
    /// Our message, not sent (failed or too long): the bar above the field
    /// offers to retry, send the original or edit it.
    NotSent,
}

/// Our message held back, for the bar above the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub key: LineKey,
    pub original: String,
    pub too_long: bool,
}

/// An emote's "/me" stays as it is; only what follows is translated.
fn split_emote(text: &str) -> (&'static str, &str) {
    if let Some(rest) = text.strip_prefix("/me ") {
        ("/me ", rest)
    } else if let Some(rest) = text.strip_prefix("/me'") {
        ("/me'", rest)
    } else {
        ("", text)
    }
}

impl Translations {
    fn job(&mut self, key: LineKey) -> Option<Job> {
        let e = self.lines.get_mut(&key)?;
        self.next += 1;
        e.job = Some(self.next);
        self.jobs.insert(self.next, key);
        Some(Job {
            id: self.next,
            text: e.body.clone(),
            from: e.from,
            to: e.to,
            direction: e.direction,
        })
    }

    /// Our message, typed in a translated conversation. Without a job (a
    /// message with nothing to translate), it still leaves in its turn.
    pub fn outgoing(&mut self, key: LineKey, text: &str, from: &'static Lang, to: &'static Lang) -> (Option<Job>, Vec<Effect>) {
        let (prefix, body) = split_emote(text);
        let skip = nothing_to_translate(body);
        self.lines.insert(
            key,
            Entry {
                state: LineState::Pending,
                job: None,
                direction: Direction::Outgoing,
                prefix,
                body: body.into(),
                from,
                to,
                started: None,
                ready: skip.then(|| text.into()),
            },
        );
        self.queues.entry(key.session).or_default().push_back(key);
        self.version += 1;
        if skip {
            return (None, self.flush(key.session));
        }
        (self.job(key), Vec::new())
    }

    /// A message received in a translated conversation; `None` when there is
    /// nothing to translate or it is already in my language.
    pub fn incoming(&mut self, key: LineKey, text: &str, partner: &'static Lang, me: &'static Lang) -> Option<Job> {
        let (prefix, body) = split_emote(text);
        if nothing_to_translate(body) || detect_between(body, me, partner).is_some_and(|d| d.lang == me && d.confidence >= MINE) {
            return None;
        }
        self.lines.insert(
            key,
            Entry {
                state: LineState::Pending,
                job: None,
                direction: Direction::Incoming,
                prefix,
                body: body.into(),
                from: partner,
                to: me,
                started: None,
                ready: None,
            },
        );
        self.version += 1;
        self.job(key)
    }

    pub fn done(&mut self, d: Done) -> Vec<Effect> {
        let Some(key) = self.jobs.remove(&d.id) else {
            return Vec::new();
        };
        let Some(e) = self.lines.get_mut(&key) else {
            return Vec::new();
        };
        if e.job != Some(d.id) {
            return Vec::new();
        }
        e.job = None;
        self.version += 1;
        let original = format!("{}{}", e.prefix, e.body);
        match (e.direction, d.result) {
            (Direction::Incoming, Ok(t)) => {
                let text = format!("{}{t}", e.prefix);
                e.state = LineState::Translated { original };
                vec![Effect::Replace { key, text }]
            }
            (Direction::Incoming, Err(err)) => {
                log::info!("incoming translation failed: {err}");
                e.state = LineState::Failed { reason: FailReason::Error };
                Vec::new()
            }
            (Direction::Outgoing, Ok(t)) => {
                let text = format!("{}{t}", e.prefix);
                if text.len() > MAX_IM_BYTES {
                    e.state = LineState::TooLong { translation: text };
                } else {
                    e.ready = Some(text);
                }
                self.flush(key.session)
            }
            (Direction::Outgoing, Err(err)) => {
                log::info!("outgoing translation failed: {err}");
                e.state = LineState::Failed { reason: FailReason::Error };
                self.flush(key.session)
            }
        }
    }

    /// Our messages whose wait is over.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let late: Vec<LineKey> = self
            .lines
            .iter()
            .filter(|(_, e)| {
                e.direction == Direction::Outgoing
                    && e.state == LineState::Pending
                    && e.ready.is_none()
                    && e.started.is_some_and(|t| now.duration_since(t) > OUTGOING_TIMEOUT)
            })
            .map(|(k, _)| *k)
            .collect();
        let mut fx = Vec::new();
        if !late.is_empty() {
            self.version += 1;
        }
        for key in late {
            if let Some(e) = self.lines.get_mut(&key) {
                e.state = LineState::Failed {
                    reason: FailReason::Timeout,
                };
                if let Some(id) = e.job.take() {
                    self.jobs.remove(&id);
                }
            }
            fx.extend(self.flush(key.session));
        }
        fx
    }

    /// Send the conversation's messages that are ready, in typed order: a
    /// message still translating holds the ones typed after it.
    fn flush(&mut self, session: Uuid) -> Vec<Effect> {
        let mut fx = Vec::new();
        let Some(q) = self.queues.get_mut(&session) else {
            return fx;
        };
        while let Some(key) = q.front().copied() {
            let Some(e) = self.lines.get_mut(&key) else {
                q.pop_front();
                continue;
            };
            if let Some(text) = e.ready.take() {
                let original = format!("{}{}", e.prefix, e.body);
                fx.push(Effect::Send {
                    to: session,
                    text: text.clone(),
                });
                if text == original {
                    self.lines.remove(&key);
                } else {
                    e.state = LineState::Translated { original };
                    fx.push(Effect::Replace { key, text });
                }
            } else {
                // still translating, or failed: the messages typed after it
                // wait for it, or for the user's choice (Retry, Send
                // original, Edit)
                break;
            }
            q.pop_front();
        }
        fx
    }

    pub fn retry(&mut self, key: LineKey) -> Option<Job> {
        let e = self.lines.get_mut(&key)?;
        if !matches!(e.state, LineState::Failed { .. }) {
            return None;
        }
        e.state = LineState::Pending;
        e.started = None;
        self.version += 1;
        self.job(key)
    }

    /// Our failed or too long message, sent as typed, then the messages
    /// that waited behind it.
    pub fn send_original(&mut self, key: LineKey) -> Vec<Effect> {
        let Some(e) = self.lines.get(&key) else {
            return Vec::new();
        };
        if e.direction != Direction::Outgoing || !matches!(e.state, LineState::Failed { .. } | LineState::TooLong { .. }) {
            return Vec::new();
        }
        let text = format!("{}{}", e.prefix, e.body);
        self.lines.remove(&key);
        self.version += 1;
        let mut fx = vec![Effect::Send { to: key.session, text }];
        fx.extend(self.flush(key.session));
        fx
    }

    /// Our held message dropped ("Edit" puts it back in the field): the
    /// messages that waited behind it leave.
    pub fn discard(&mut self, key: LineKey) -> Vec<Effect> {
        if self.lines.remove(&key).is_none() {
            return Vec::new();
        }
        self.version += 1;
        self.flush(key.session)
    }

    /// The model began a job: our message's timeout starts now.
    pub fn started(&mut self, job: u64, now: Instant) {
        if let Some(e) = self.jobs.get(&job).and_then(|k| self.lines.get_mut(k))
            && e.started.is_none()
        {
            e.started = Some(now);
        }
    }

    /// The translator died: nothing pending will come back.
    pub fn fail_pending(&mut self) {
        for e in self
            .lines
            .values_mut()
            .filter(|e| e.state == LineState::Pending && e.ready.is_none())
        {
            e.state = LineState::Failed { reason: FailReason::Error };
            e.job = None;
        }
        self.jobs.clear();
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// What every line shows, by the time of its line, and our held
    /// messages, oldest first.
    pub fn view(&self) -> (HashMap<SystemTime, LineShow>, Vec<Held>) {
        let mut held = Vec::new();
        let lines = self
            .lines
            .iter()
            .map(|(k, e)| {
                let show = match &e.state {
                    LineState::Pending => LineShow::Pending,
                    LineState::Translated { original } => LineShow::Original(original.clone()),
                    LineState::Failed { .. } | LineState::TooLong { .. } if e.direction == Direction::Outgoing => {
                        held.push(Held {
                            key: *k,
                            original: format!("{}{}", e.prefix, e.body),
                            too_long: matches!(e.state, LineState::TooLong { .. }),
                        });
                        LineShow::NotSent
                    }
                    LineState::Failed { .. } | LineState::TooLong { .. } => LineShow::NotTranslated,
                };
                (k.time, show)
            })
            .collect();
        held.sort_by_key(|h| h.key.time);
        (lines, held)
    }

    #[cfg(test)]
    pub fn state(&self, key: &LineKey) -> Option<&LineState> {
        self.lines.get(key).map(|e| &e.state)
    }

    /// The text typed or received, for "Edit" and the swap of a line.
    pub fn original(&self, key: &LineKey) -> Option<String> {
        self.lines.get(key).map(|e| format!("{}{}", e.prefix, e.body))
    }

    pub fn close_session(&mut self, session: Uuid) {
        self.lines.retain(|k, _| k.session != session);
        self.version += 1;
        self.jobs.retain(|_, k| k.session != session);
        self.queues.remove(&session);
    }

    pub fn clear(&mut self) {
        *self = Self {
            next: self.next,
            ..Self::default()
        };
    }
}

/// "They seem to write Spanish": two messages in a row, long enough, in the
/// same language that is not mine.
#[derive(Debug, Default)]
pub struct Suggest {
    runs: HashMap<Uuid, (&'static Lang, u8)>,
}

impl Suggest {
    pub fn observe(&mut self, session: Uuid, text: &str, me: &'static Lang) -> Option<&'static Lang> {
        let found = (text.chars().count() >= SUGGEST_MIN_CHARS)
            .then(|| detect(text))
            .flatten()
            .filter(|d| d.confidence >= SUGGEST_CONFIDENCE && d.lang != me)
            .map(|d| d.lang);
        let Some(lang) = found else {
            self.runs.remove(&session);
            return None;
        };
        let run = self.runs.entry(session).or_insert((lang, 0));
        if run.0 != lang {
            *run = (lang, 0);
        }
        run.1 += 1;
        if run.1 >= 2 {
            self.runs.remove(&session);
            return Some(lang);
        }
        None
    }
}
