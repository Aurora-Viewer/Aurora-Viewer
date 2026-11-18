//! A message of a direct conversation as the history stitcher sees it,
//! whatever its source: a Chat Service row, a line of the legacy plaintext
//! transcript, or a live row of the open session. The LL viewer keeps these
//! as LLSD maps (LLIMSession::mMsgs); here the keys the stitcher reads are
//! typed fields and every other key of the session model rides in `extra`.
//! Also the capture of a live message's time provenance (incomingContext,
//! captureLiveMessage, recordLiveMessage; indra/newview/
//! llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1).
//!
//! What the source of a row is:
//! - a service row has `msg_id` (the LL viewer's "a row is a service row
//!   iff `chat_service_msg_id` is a string");
//! - a plaintext transcript line has neither `msg_id` nor `timestamp`, and
//!   `legacy_wall_time` when its SLT date and time could be read;
//! - a live row has `is_history` false and the capture fields below.

use aurora_llsd::Map;
use uuid::Uuid;

/// Which clock gave a live message its `timestamp`
/// (`chat_service_time_source`), and so how closely a service row's time
/// must agree with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSource {
    /// The sender's send time, saved with an offline message ("sent"):
    /// the service row has the same second.
    Sent,
    /// An online delivery stamped on receipt ("receipt"): the service may
    /// record it up to 5 s later or 60 s earlier.
    Receipt,
    /// This computer's clock (outgoing echoes, offline messages without a
    /// send time; "local"): within 59 s either way.
    Local,
    /// A caller that names no clock (an empty string): exact seconds, like
    /// `Sent`, but a later capture does not replace it.
    Unmarked,
}

/// One message (an LLSD map of LLIMSession::mMsgs in the LL viewer). The
/// equality is the LL viewer's llsd_equals over the whole map.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryMessage {
    /// The sender's name as shown or logged ("from").
    pub from: String,
    /// The sender, when known ("from_id"): it decides the sender match over
    /// the name when both rows have one.
    pub from_id: Option<Uuid>,
    /// The body as displayed and logged, with an offline notice prefix or a
    /// translation ("message").
    pub message: String,
    /// The presentation time ("time").
    pub time: String,
    /// The transcript's SLT date and time, or a service row's ISO
    /// `created_at` ("datetime").
    pub datetime: String,
    /// UTC seconds ("timestamp"); plaintext lines have none, an untimed
    /// live row (an inline offer) has none or 0.
    pub timestamp: Option<i32>,
    /// The presentation index, oldest 0 ("index"); never compared when the
    /// history is republished.
    pub index: Option<i32>,
    pub is_history: bool,
    pub is_region_msg: bool,
    /// The service message id (`chat_service_msg_id`).
    pub msg_id: Option<String>,
    /// A plaintext line's SLT wall time read as if it were UTC, in seconds
    /// (`chat_service_legacy_wall_time`): the true UTC is 7 or 8 h later.
    pub legacy_wall_time: Option<f64>,
    /// A live row's body before the offline notice and the translation
    /// (`chat_service_original_text`): what the service stores.
    pub original_text: Option<String>,
    /// `chat_service_time_source`.
    pub time_source: Option<TimeSource>,
    /// UTC seconds when a live row was recorded, about when its plaintext
    /// line was written (`chat_service_log_timestamp`).
    pub log_timestamp: Option<i32>,
    /// The newest service id known before an outgoing message was sent
    /// (`chat_service_after_msg_id`), possibly empty: its service copy is
    /// newer than that.
    pub after_msg_id: Option<String>,
    /// Every other key of the session model (notification ids, offer data),
    /// compared like the rest.
    pub extra: Map,
}

impl HistoryMessage {
    /// A service row (`chat_service_msg_id` is a string).
    pub fn is_service(&self) -> bool {
        self.msg_id.is_some()
    }
}

/// What a live message carries from its arrival to its recording, through
/// translation (the LL viewer's history context map).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveContext {
    pub original_text: Option<String>,
    pub time_source: Option<TimeSource>,
    pub after_msg_id: Option<String>,
}

/// The context of an incoming direct message: its raw packet body (`None`
/// when the offline notice was prepended) and its clock, the receipt time
/// online, the saved send time offline. Port of
/// LLChatServiceHistoryCore::incomingContext.
pub fn incoming_context(original_text: Option<&str>, online: bool) -> LiveContext {
    LiveContext {
        original_text: original_text.map(str::to_owned),
        time_source: Some(if online { TimeSource::Receipt } else { TimeSource::Sent }),
        after_msg_id: None,
    }
}

/// Capture a live message's timing before it is translated: the original
/// body if the context has none, the clock if none is named or a missing
/// timestamp leaves only this computer's (a receipt keeps its own), and a
/// zero `timestamp` becomes `now` in place. Calling it again is harmless.
/// Port of LLChatServiceHistoryCore::captureLiveMessage.
pub fn capture_live_message(text: &str, timestamp: &mut u32, context: &LiveContext, now: u32) -> LiveContext {
    let mut captured = context.clone();
    if captured.original_text.is_none() {
        captured.original_text = Some(text.to_owned());
    }
    if context.time_source.is_none() || (*timestamp == 0 && context.time_source != Some(TimeSource::Receipt)) {
        captured.time_source = Some(if *timestamp != 0 { TimeSource::Sent } else { TimeSource::Local });
    }
    if *timestamp == 0 {
        *timestamp = now;
    }
    captured
}

/// Record the capture on the session row once it is logged, with the time
/// it was logged; the pre-send boundary is kept through translation and
/// later reloads. Port of LLChatServiceHistoryCore::recordLiveMessage.
pub fn record_live_message(message: &mut HistoryMessage, context: &LiveContext, logged_at: u32) {
    message.original_text.clone_from(&context.original_text);
    message.time_source = context.time_source;
    message.log_timestamp = Some(logged_at as i32);
    if context.after_msg_id.is_some() {
        message.after_msg_id.clone_from(&context.after_msg_id);
    }
}

/// Whether a legacy line may come before the service archive's oldest
/// row: its SLT wall time read at its earliest possible UTC (UTC-7, as no
/// offset is stored), strictly before. Port of
/// LLChatServiceHistoryCore::legacyWallMayPrecedeService.
pub fn legacy_wall_may_precede_service(wall_epoch: f64, service_epoch: f64) -> bool {
    wall_epoch + 7.0 * 3600.0 < service_epoch
}
