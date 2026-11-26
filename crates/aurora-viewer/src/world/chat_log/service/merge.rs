//! The stitcher of a direct conversation's history: the service archive,
//! the legacy plaintext transcript and the live rows of the open session,
//! shown together with every message once. Port of the composition half of
//! LLChatServiceHistoryCore (indra/newview/llchatservicehistorycore.cpp,
//! LL viewer 26.5, LGPL 2.1).
//!
//! The same message can reach the view three ways: a service row (exact
//! UTC seconds, original body), a plaintext line (SLT minute of logging,
//! displayed body) and a live row (one of three clocks, see
//! [`TimeSource`]). Nothing identifies them for sure, so each copy is
//! consumed one-for-one by the evidence that fits its clock, and the
//! choices among equal candidates are deterministic: the tie-breaks below
//! are the LL viewer's and decide which service id stays on screen.
//!
//! Lists are oldest-first except the session model and the live rows,
//! which are newest-first like LLIMSession::mMsgs.

use super::message::{HistoryMessage, TimeSource};
use aurora_net::chat_service::{TimeUuidKey, parse_time_uuid};
use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, VecDeque};
use uuid::Uuid;

/// A list of messages; see the module comment for its order.
pub type Messages = Vec<HistoryMessage>;

/// A local-clock echo and its service row may be this far apart, either way.
const MAX_LOCAL_CLOCK_SKEW: i64 = 59;
/// An online receipt may be stamped this long after the service recorded it.
const MAX_DELIVERY_DELAY: i64 = 60;
/// ... or this long before.
const MAX_SERVICE_LEAD: i64 = 5;
/// SLT is UTC-7 (PDT) or UTC-8 (PST); a transcript stores neither, so both
/// are always tried.
const SLT_OFFSETS: [i64; 2] = [7 * 3600, 8 * 3600];

fn service_key(row: &HistoryMessage) -> Option<TimeUuidKey> {
    row.msg_id.as_deref().and_then(parse_time_uuid)
}

/// The LL viewer groups rows by `from_id.asUUID()` (null when missing) and
/// body.
fn identity(row: &HistoryMessage) -> (Uuid, String) {
    (row.from_id.unwrap_or_default(), row.message.clone())
}

/// LLCacheName::buildLegacyName (indra/llmessage/llcachename.cpp): "Disp
/// (first.last)" gives "First Last", "Disp (single)" gives "Single", any
/// other name is kept. Only the first byte of each part is uppercased, and
/// only when ASCII, as the LL viewer's byte-wise toupper does.
fn build_legacy_name(complete: &str) -> String {
    let capitalised = |part: &str| {
        let mut part = part.to_owned();
        if let Some(first) = part.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        part
    };
    if let (Some(open), Some(close)) = (complete.rfind(" ("), complete.rfind(')'))
        && close == complete.len() - 1
        && close >= open + 2
    {
        let inner = &complete[open + 2..close];
        if !inner.is_empty() {
            let legacy = capitalised(inner);
            return match legacy.split_once('.') {
                Some((first, last)) if !last.is_empty() => format!("{first} {}", capitalised(last)),
                Some((first, _)) => first.to_owned(),
                None => legacy,
            };
        }
    }
    complete.to_owned()
}

/// LLCacheName::buildUsername: "First Last" gives "first.last", "First
/// Resident" gives "first"; a name without a space is only lowercased
/// (cleanFullName finds no " Resident" in it). ASCII case only, as the
/// LL viewer's byte-wise tolower.
fn build_username(full_name: &str) -> String {
    if full_name == "(???) (???)" {
        return "(???)".to_owned();
    }
    match full_name.split_once(' ') {
        Some((first, "Resident")) => first.to_ascii_lowercase(),
        Some((first, last)) => format!("{first}.{last}").to_ascii_lowercase(),
        None => full_name.to_ascii_lowercase(),
    }
}

/// Whether two transcript names are the same resident: the username inside
/// a complete name, legacy and dotted spellings and case all agree; the
/// display name is presentation only, and an empty name matches nothing.
/// Port of LLChatServiceHistoryCore::sameDirectSenderName.
pub fn same_direct_sender_name(left: &str, right: &str) -> bool {
    let left = build_username(&build_legacy_name(left));
    !left.is_empty() && left == build_username(&build_legacy_name(right))
}

/// Whether `timed` (a live row, or a service row during a seam merge) is
/// an occurrence of the stored row `history`. A service row is compared
/// with the original body and by the clock of `timed` (receipt -5/+60 s,
/// local ±59 s and newer than its pre-send boundary, otherwise the exact
/// second); a plaintext line with the displayed body, by the SLT minute
/// `timed` was logged in under either Pacific offset. Port of
/// LLChatServiceHistoryCore::sameDirectHistoryOccurrence.
pub fn same_direct_history_occurrence(history: &HistoryMessage, timed: &HistoryMessage) -> bool {
    let Some(timestamp) = timed.timestamp.filter(|t| *t > 0) else {
        return false;
    };
    // Service bodies omit offline notices and translations; plaintext has
    // the rendered body, so each is compared with the matching value.
    let service = history.is_service();
    let text = match &timed.original_text {
        Some(original) if service => original,
        _ => &timed.message,
    };
    if history.message != *text {
        return false;
    }
    let same_sender = match (history.from_id, timed.from_id) {
        (Some(left), Some(right)) => left == right,
        _ => same_direct_sender_name(&history.from, &timed.from),
    };
    if !same_sender {
        return false;
    }

    let timestamp = i64::from(timestamp);
    if service {
        let Some(service_timestamp) = history.timestamp.filter(|t| *t > 0).map(i64::from) else {
            return false;
        };
        let skew = timestamp - service_timestamp;
        return match timed.time_source {
            // Receipt and service recording use separate clocks and can
            // finish in either order.
            Some(TimeSource::Receipt) => (-MAX_SERVICE_LEAD..=MAX_DELIVERY_DELAY).contains(&skew),
            Some(TimeSource::Local) => {
                // History known before this send cannot be its service copy.
                if let Some(after) = timed.after_msg_id.as_deref().and_then(parse_time_uuid)
                    && service_key(history).is_none_or(|key| key <= after)
                {
                    return false;
                }
                (-MAX_LOCAL_CLOCK_SKEW..=MAX_LOCAL_CLOCK_SKEW).contains(&skew)
            }
            _ => skew == 0,
        };
    }

    // Offline IMs are logged when delivered, possibly days after they were
    // sent: the line's minute is the logging minute.
    let Some(wall) = history.legacy_wall_time else {
        return false;
    };
    let logged = f64::from(timed.log_timestamp.unwrap_or(timestamp as i32));
    SLT_OFFSETS.iter().any(|&offset| {
        let minute = wall + offset as f64;
        logged >= minute && logged < minute + 60.0
    })
}

/// A timed live or service occurrence within one sender/body group.
#[derive(Debug, Clone, Copy)]
struct TimedOccurrence {
    timestamp: i64,
    /// The delivery's position, or the service row's.
    index: usize,
    key: TimeUuidKey,
}

/// The band of the alignment table: a maximum-count path skips at most
/// `skip_local` and `skip_service` occurrences.
struct Band {
    skip_local: usize,
    skip_service: usize,
    columns: usize,
}

/// (pairs, minus the total skew): more pairs first, then less skew.
type Score = (i32, i32);

impl Band {
    fn begin(&self, row: usize) -> usize {
        row.saturating_sub(self.skip_local)
    }

    fn end(&self, row: usize) -> usize {
        self.columns.min(row + self.skip_service)
    }

    /// Cells outside the band are unreachable for a maximum-count path.
    fn at(&self, scores: &[Vec<Score>], row: usize, column: usize) -> Score {
        let begin = self.begin(row);
        if column >= begin && column <= self.end(row) {
            scores[row][column - begin]
        } else {
            (-1, 0)
        }
    }

    fn paired(&self, scores: &[Vec<Score>], local: &[TimedOccurrence], service: &[TimedOccurrence], row: usize, column: usize) -> Score {
        let distance = (local[row].timestamp - service[column].timestamp).abs();
        let suffix = self.at(scores, row + 1, column + 1);
        if distance <= MAX_LOCAL_CLOCK_SKEW && suffix.0 >= 0 {
            (suffix.0 + 1, suffix.1 - distance as i32)
        } else {
            (-1, 0)
        }
    }
}

/// Pair the local echoes and service rows of one sender/body/time
/// component, both newest-first: the most pairs, then the least total
/// skew. The ordered greedy pass gives the maximum count (and is the answer
/// when it pairs everything); a DP within the band it implies then picks
/// the least skew, ties going to the newer pair, then to skipping a service
/// row. Port of matchLocalOccurrences (llchatservicehistorycore.cpp).
fn match_local_occurrences(local: &[TimedOccurrence], service: &[TimedOccurrence], pairs: &mut Vec<(usize, usize)>) {
    let saved = pairs.len();
    let (mut i, mut j) = (0, 0);
    while i < local.len() && j < service.len() {
        let skew = local[i].timestamp - service[j].timestamp;
        if skew.abs() <= MAX_LOCAL_CLOCK_SKEW {
            pairs.push((local[i].index, service[j].index));
            i += 1;
            j += 1;
        } else if skew > 0 {
            i += 1;
        } else {
            j += 1;
        }
    }
    let count = pairs.len() - saved;
    if count == 0 || (count == local.len() && count == service.len()) {
        return;
    }
    pairs.truncate(saved);

    // Suffix scores, stored only inside the band; they also give the
    // traceback. Rows and columns at the far edges keep (0, 0).
    let band = Band {
        skip_local: local.len() - count,
        skip_service: service.len() - count,
        columns: service.len(),
    };
    let mut scores: Vec<Vec<Score>> = vec![Vec::new(); local.len() + 1];
    for row in (0..=local.len()).rev() {
        let (begin, end) = (band.begin(row), band.end(row));
        scores[row] = vec![(0, 0); end - begin + 1];
        for column in (begin..=end).rev() {
            if row < local.len() && column < service.len() {
                let best = band
                    .at(&scores, row + 1, column)
                    .max(band.at(&scores, row, column + 1))
                    .max(band.paired(&scores, local, service, row, column));
                scores[row][column - begin] = best;
            }
        }
    }

    let (mut i, mut j) = (0, 0);
    while i < local.len() && j < service.len() {
        let here = band.at(&scores, i, j);
        if here == band.paired(&scores, local, service, i, j) {
            pairs.push((local[i].index, service[j].index));
            i += 1;
            j += 1;
        } else if here == band.at(&scores, i, j + 1) {
            j += 1;
        } else {
            i += 1;
        }
    }
}

/// Where a capture's local echoes are matched: the newest pre-send
/// boundary first, captures with no boundary last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CaptureOrder {
    unbounded: bool,
    boundary: Reverse<TimeUuidKey>,
}

/// The live and service occurrences of one sender and body.
#[derive(Default)]
struct Group {
    captures: BTreeMap<CaptureOrder, Vec<TimedOccurrence>>,
    /// Newest first.
    service: Vec<TimedOccurrence>,
    cursor: usize,
    receipts: bool,
}

/// A row of `candidates` (newest-first) within the local-clock skew of `row`.
fn has_candidate(row: &TimedOccurrence, candidates: &[TimedOccurrence]) -> bool {
    let first = candidates.partition_point(|c| c.timestamp > row.timestamp + MAX_LOCAL_CLOCK_SKEW);
    candidates
        .get(first)
        .is_some_and(|c| c.timestamp >= row.timestamp - MAX_LOCAL_CLOCK_SKEW)
}

/// The (delivery, service row) pairs: exact sends, then receipts, each
/// newest-first and taking the newest eligible row of its group, then the
/// local echoes by capture boundary, newest first, through
/// [`match_local_occurrences`] on each separate time component. A newer
/// capture's first claim can leave a duplicate when clock windows compete:
/// assignments are not optimised across captures. Deliveries or service
/// rows without a sender id fall back to a scan in service order with the
/// sender names. Port of serviceMatches (llchatservicehistorycore.cpp).
fn service_matches(deliveries: &[&HistoryMessage], service: &[&HistoryMessage]) -> Vec<(usize, usize)> {
    let mut groups: BTreeMap<(Uuid, String), Group> = BTreeMap::new();
    let complete_ids = service.iter().all(|row| row.from_id.is_some());
    for (pos, row) in service.iter().enumerate() {
        if let Some(timestamp) = row.timestamp.filter(|t| *t > 0) {
            let key = service_key(row).unwrap_or_default();
            groups.entry(identity(row)).or_default().service.push(TimedOccurrence {
                timestamp: i64::from(timestamp),
                index: pos,
                key,
            });
        }
    }

    let mut used = vec![false; service.len()];
    let mut pairs = Vec::new();
    for (pos, delivery) in deliveries.iter().enumerate() {
        let Some(timestamp) = delivery.timestamp.filter(|t| *t > 0).map(i64::from) else {
            continue;
        };
        let local = delivery.time_source == Some(TimeSource::Local);
        let Some(from_id) = delivery.from_id.filter(|_| local || complete_ids) else {
            if let Some(row) = (0..service.len()).find(|&row| !used[row] && same_direct_history_occurrence(service[row], delivery)) {
                pairs.push((pos, row));
                used[row] = true;
            }
            continue;
        };
        let body = delivery.original_text.as_ref().unwrap_or(&delivery.message);
        let Some(group) = groups.get_mut(&(from_id, body.clone())) else {
            continue;
        };
        if local {
            let after = delivery.after_msg_id.as_deref().and_then(parse_time_uuid).unwrap_or_default();
            let order = CaptureOrder {
                unbounded: after == TimeUuidKey::default(),
                boundary: Reverse(after),
            };
            group.captures.entry(order).or_default().push(TimedOccurrence {
                timestamp,
                index: pos,
                key: TimeUuidKey::default(),
            });
            continue;
        }

        // Each cursor passes a candidate at most once per policy; receipts
        // start again after the exact sends.
        let receipt = delivery.time_source == Some(TimeSource::Receipt);
        if receipt && !group.receipts {
            group.cursor = 0;
            group.receipts = true;
        }
        let latest = timestamp + if receipt { MAX_SERVICE_LEAD } else { 0 };
        let earliest = timestamp - if receipt { MAX_DELIVERY_DELAY } else { 0 };
        while group
            .service
            .get(group.cursor)
            .is_some_and(|row| row.timestamp > latest || used[row.index])
        {
            group.cursor += 1;
        }
        if let Some(row) = group.service.get(group.cursor).filter(|row| row.timestamp >= earliest) {
            pairs.push((pos, row.index));
            used[row.index] = true;
            group.cursor += 1;
        }
    }

    for group in groups.values_mut() {
        for (capture, local_rows) in group.captures.iter_mut() {
            let (Some(newest), Some(oldest)) = (local_rows.first(), local_rows.last()) else {
                continue;
            };
            // Only the service rows this capture's timestamps can reach,
            // unclaimed and newer than its pre-send boundary.
            let first = group
                .service
                .partition_point(|row| row.timestamp > newest.timestamp + MAX_LOCAL_CLOCK_SKEW);
            let reach = group.service[first..].partition_point(|row| row.timestamp >= oldest.timestamp - MAX_LOCAL_CLOCK_SKEW);
            let mut service_rows = group.service[first..first + reach].to_vec();
            service_rows.retain(|row| !used[row.index] && (capture.unbounded || row.key > capture.boundary.0));
            let saved = pairs.len();
            local_rows.retain(|row| has_candidate(row, &service_rows));
            if local_rows.is_empty() {
                continue;
            }
            service_rows.retain(|row| has_candidate(row, local_rows));
            let (local, rows) = (&local_rows[..], &service_rows[..]);

            // Time regions further apart than the skew cannot share a
            // candidate, nor sway each other's tied choices.
            let (mut i, mut j, mut local_start, mut service_start) = (0, 0, 0, 0);
            let mut previous = local[0].timestamp.max(rows[0].timestamp);
            while i < local.len() || j < rows.len() {
                let timestamp = local.get(i).map_or(0, |r| r.timestamp).max(rows.get(j).map_or(0, |r| r.timestamp));
                if previous - timestamp > MAX_LOCAL_CLOCK_SKEW {
                    match_local_occurrences(&local[local_start..i], &rows[service_start..j], &mut pairs);
                    local_start = i;
                    service_start = j;
                }
                if local.get(i).is_some_and(|r| r.timestamp == timestamp) {
                    i += 1;
                } else {
                    j += 1;
                }
                previous = timestamp;
            }
            match_local_occurrences(&local[local_start..], &rows[service_start..], &mut pairs);

            // A selected row is no longer available to the next capture.
            for &(_, row) in &pairs[saved..] {
                used[row] = true;
            }
        }
    }
    pairs
}

/// The smallest and largest of a time field, (f64::MAX, 0) when no row has
/// it: a broad window that excludes old traffic cheaply (historyTimeRange).
fn time_range(times: impl Iterator<Item = Option<f64>>) -> (f64, f64) {
    times.flatten().fold((f64::MAX, 0.0), |(low, high), t| (low.min(t), high.max(t)))
}

/// A consumed stored row and the live row that consumed it.
type Match = (HistoryMessage, HistoryMessage);

/// `history` without the rows the live rows already show, in its order.
/// Service rows go first (see [`service_matches`]); then each delivery may
/// also consume one plaintext line, unless that line could itself be the
/// logged copy of the service row the delivery took, which makes it an
/// extra occurrence. Port of filterHistoryDuplicates
/// (llchatservicehistorycore.cpp).
fn filter_history_duplicates(history: &[HistoryMessage], live: &[HistoryMessage], mut matches: Option<&mut Vec<Match>>) -> Messages {
    if history.is_empty() || live.is_empty() {
        return history.to_vec();
    }

    // Skip old session traffic outside both clock windows before sorting;
    // plaintext uses the logging time under either Pacific offset.
    let service_time = time_range(history.iter().map(|row| row.timestamp.map(f64::from)));
    let legacy_time = time_range(history.iter().map(|row| row.legacy_wall_time));
    let mut deliveries: Vec<&HistoryMessage> = live
        .iter()
        .filter(|message| {
            let timestamp = message.timestamp.unwrap_or(0);
            let logged = f64::from(message.log_timestamp.unwrap_or(timestamp));
            let timestamp = f64::from(timestamp);
            (timestamp >= service_time.0 - MAX_LOCAL_CLOCK_SKEW as f64 && timestamp <= service_time.1 + MAX_DELIVERY_DELAY as f64)
                || (logged >= legacy_time.0 + 7.0 * 3600.0 && logged < legacy_time.1 + 8.0 * 3600.0 + 60.0)
        })
        .collect();
    // Known sends, then receipts, then local echoes; newest first in each.
    let priority = |message: &HistoryMessage| match message.time_source {
        Some(TimeSource::Receipt) => 1,
        Some(TimeSource::Local) => 2,
        _ => 0,
    };
    deliveries.sort_by(|a, b| {
        priority(a)
            .cmp(&priority(b))
            .then(b.timestamp.unwrap_or(0).cmp(&a.timestamp.unwrap_or(0)))
    });

    let mut service: Vec<usize> = (0..history.len()).filter(|&row| history[row].is_service()).collect();
    service.sort_by_key(|&row| Reverse(history[row].timestamp.unwrap_or(0)));
    let service_rows: Vec<&HistoryMessage> = service.iter().map(|&row| &history[row]).collect();

    let mut removed = vec![false; history.len()];
    let mut matched: Vec<Option<usize>> = vec![None; deliveries.len()];
    for (delivery, row) in service_matches(&deliveries, &service_rows) {
        let row = service[row];
        matched[delivery] = Some(row);
        if let Some(matches) = matches.as_deref_mut() {
            matches.push((history[row].clone(), deliveries[delivery].clone()));
        }
        removed[row] = true;
    }

    // Plaintext the matched service row could not consume: a decorated
    // body, or another actual logging minute.
    if service.len() < history.len() {
        for (pos, delivery) in deliveries.iter().enumerate() {
            let found = (0..history.len()).find(|&row| {
                let line = &history[row];
                !removed[row]
                    && !line.is_service()
                    && same_direct_history_occurrence(line, delivery)
                    && matched[pos].is_none_or(|taken| !same_direct_history_occurrence(line, &history[taken]))
            });
            if let Some(row) = found {
                if let Some(matches) = matches.as_deref_mut() {
                    matches.push((history[row].clone(), (*delivery).clone()));
                }
                removed[row] = true;
            }
        }
    }
    history
        .iter()
        .zip(removed)
        .filter(|(_, removed)| !removed)
        .map(|(row, _)| row.clone())
        .collect()
}

/// `history` (oldest-first) without the rows `live` already shows. Port of
/// LLChatServiceHistoryCore::filterDirectHistoryDuplicates.
#[cfg(test)]
pub fn filter_direct_history_duplicates(history: &[HistoryMessage], live: &[HistoryMessage]) -> Messages {
    filter_history_duplicates(history, live, None)
}

/// Keep, ahead of the visible rows, the consumed occurrences that later
/// compositions need to consume the same rows again once the archive
/// window has moved on: plaintext matches whose live row still covers a
/// retained line (walked backwards, as the two SLT offsets chain them),
/// and every service match of a connected cluster (timestamps within twice
/// the skew) that protects a retained row or a loaded plaintext line. Port
/// of retainHistoryAnchors (llchatservicehistorycore.cpp).
fn retain_history_anchors(retained: &mut Messages, matches: &[Match], loaded: &[HistoryMessage]) {
    let mut rows: VecDeque<HistoryMessage> = std::mem::take(retained).into();
    // Per sender and body, (timestamp, match) sorted by time, equal times in
    // reverse match order (the LL viewer's multimap, filled backwards).
    let mut groups: BTreeMap<(Uuid, String), Vec<(i32, usize)>> = BTreeMap::new();
    for (pos, (row, live)) in matches.iter().enumerate().rev() {
        if !row.is_service() {
            if rows
                .iter()
                .any(|candidate| !candidate.is_service() && same_direct_history_occurrence(candidate, live))
            {
                rows.push_front(row.clone());
            }
            continue;
        }
        groups.entry(identity(row)).or_default().push((row.timestamp.unwrap_or(0), pos));
    }

    // The context is what `rows` holds now; service anchors go in front of
    // it, so it is everything after the `anchors` first rows.
    let mut anchors = 0;
    for times in groups.values_mut() {
        times.sort_by_key(|&(timestamp, _)| timestamp);
        let mut begin = 0;
        while begin < times.len() {
            let mut end = begin + 1;
            while end < times.len() && i64::from(times[end].0) - i64::from(times[end - 1].0) <= 2 * MAX_LOCAL_CLOCK_SKEW {
                end += 1;
            }
            let needed = times[begin..end].iter().any(|&(_, pos)| {
                let (row, live) = &matches[pos];
                rows.iter()
                    .skip(anchors)
                    .any(|context| same_direct_history_occurrence(context, live))
                    || loaded
                        .iter()
                        .any(|line| !line.is_service() && same_direct_history_occurrence(line, row))
            });
            if needed {
                for &(_, pos) in &times[begin..end] {
                    rows.push_front(matches[pos].0.clone());
                    anchors += 1;
                }
            }
            begin = end;
        }
    }
    *retained = rows.into();
}

/// Insert newest-first `history` into newest-first `live` by UTC time:
/// both keep their order, an untimed live row (an inline offer) is never
/// passed, a live row stays newer on a tie, and untimed history goes after
/// every live row. Port of LLChatServiceHistoryCore::interleaveDirectHistory.
pub fn interleave_direct_history(history: Messages, live: Messages) -> Messages {
    let mut result = Vec::with_capacity(history.len() + live.len());
    let mut live = live.into_iter().peekable();
    for message in history {
        let timestamp = message.timestamp.unwrap_or(0);
        while let Some(row) = live.next_if(|row| {
            let at = row.timestamp.unwrap_or(0);
            timestamp <= 0 || at <= 0 || at >= timestamp
        }) {
            result.push(row);
        }
        result.push(message);
    }
    result.extend(live);
    result
}

/// One side of a seam merge: plaintext (and unparseable ids) in order, and
/// service rows by key, the first of a repeated key winning.
struct Seam {
    legacy: Messages,
    service: BTreeMap<TimeUuidKey, HistoryMessage>,
}

impl Seam {
    fn of(messages: &[HistoryMessage]) -> Seam {
        let mut seam = Seam {
            legacy: Vec::new(),
            service: BTreeMap::new(),
        };
        for row in messages {
            match service_key(row) {
                Some(key) => {
                    seam.service.entry(key).or_insert_with(|| row.clone());
                }
                None => seam.legacy.push(row.clone()),
            }
        }
        seam
    }
}

/// Consume from `candidates` the service rows of every part a line longer
/// than one IM was sent as (`aurora_net::message_split`, one row per part),
/// all or none; false when the line left whole or a part has no row.
/// Aurora sends a long IM in parts like the Legacy Viewer
/// (LLIMModel::sendMessage, indra/newview/llimview.cpp), while its
/// transcript keeps the line whole.
fn consume_parts(line: &HistoryMessage, candidates: &mut Vec<&HistoryMessage>) -> bool {
    let parts = aurora_net::message_split::message_parts(&line.message);
    if parts.len() < 2 {
        return false;
    }
    let mut found: Vec<usize> = Vec::with_capacity(parts.len());
    for part in parts {
        let piece = HistoryMessage {
            message: part.to_owned(),
            ..line.clone()
        };
        let Some(pos) = (0..candidates.len()).find(|i| !found.contains(i) && same_direct_history_occurrence(&piece, candidates[*i])) else {
            return false;
        };
        found.push(pos);
    }
    found.sort_unstable();
    for pos in found.into_iter().rev() {
        candidates.remove(pos);
    }
    true
}

/// Drop the plaintext lines of one side that the other side's new service
/// rows cover, one line per row, or a long sent line per the rows of its
/// parts ([`consume_parts`]); the rows are indexed by minute so that a
/// whole transcript is not compared pair by pair.
fn reconcile(legacy: &mut Messages, own: &BTreeMap<TimeUuidKey, HistoryMessage>, other: &BTreeMap<TimeUuidKey, HistoryMessage>) {
    let mut added: BTreeMap<i64, Vec<&HistoryMessage>> = BTreeMap::new();
    for (key, row) in other {
        if !own.contains_key(key) {
            added.entry(i64::from(row.timestamp.unwrap_or(0) / 60)).or_default().push(row);
        }
    }
    legacy.retain(|line| {
        let Some(wall) = line.legacy_wall_time else {
            return true;
        };
        for offset in SLT_OFFSETS {
            let minute = (wall as i64 + offset) / 60;
            if let Some(candidates) = added.get_mut(&minute) {
                if let Some(found) = candidates.iter().position(|row| same_direct_history_occurrence(line, row)) {
                    candidates.remove(found);
                    return false;
                }
                if consume_parts(line, candidates) {
                    return false;
                }
            }
        }
        true
    });
}

/// Union two oldest-first seams, each already reconciled with its own
/// service ids: only the ids one side lacks consume its plaintext; equal
/// plaintext lines count once (a repeated line keeps the larger count of
/// the two windows) and are sorted by wall time, undated ones last; then
/// every service row in TimeUUID order, the loaded side's value winning.
/// Port of LLChatServiceHistoryCore::mergeDirectHistory.
pub fn merge_direct_history(loaded: &[HistoryMessage], incoming: &[HistoryMessage]) -> Messages {
    let mut left = Seam::of(loaded);
    let mut right = Seam::of(incoming);
    reconcile(&mut left.legacy, &left.service, &right.service);
    reconcile(&mut right.legacy, &right.service, &left.service);

    let mut unmatched = left.legacy.clone();
    for row in right.legacy {
        match unmatched.iter().position(|candidate| *candidate == row) {
            Some(found) => {
                unmatched.remove(found);
            }
            None => left.legacy.push(row),
        }
    }
    left.legacy.sort_by(|a, b| match (a.legacy_wall_time, b.legacy_wall_time) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });

    for (key, row) in right.service {
        left.service.entry(key).or_insert(row);
    }
    left.legacy.extend(left.service.into_values());
    left.legacy
}

/// The history of one view (an IM session or a conversation preview).
/// Port of LLChatServiceHistoryCore::History.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// The last stitched read from disk.
    loaded: Messages,
    /// The visible rows and the consumed anchors that keep their counts,
    /// only while the view is shown; anchors never reach the display.
    visible: Messages,
    /// The last seam before filtering, rows live messages already show
    /// included.
    candidates: Messages,
}

impl History {
    /// A new disk read; an empty one also drops the retained context.
    pub fn set_loaded(&mut self, messages: Messages) {
        if messages.is_empty() {
            self.visible.clear();
        }
        self.loaded = messages;
    }

    /// Forget everything (the user deleted the history).
    pub fn clear(&mut self) {
        self.loaded.clear();
        self.visible.clear();
        self.candidates.clear();
    }

    /// Forget the service rows and keep the plaintext (service consent
    /// withdrawn).
    pub fn clear_service(&mut self) {
        for list in [&mut self.loaded, &mut self.visible, &mut self.candidates] {
            list.retain(|row| !row.is_service());
        }
    }

    /// The history to show (oldest-first, at most `limit` rows, 0 for no
    /// limit): the loaded read, the service preview and, while the view is
    /// shown (`retain_context`), the context it showed last, without what
    /// `live` (newest-first) shows. Merging before filtering keeps a live
    /// copy from evicting retained context.
    pub fn compose(&mut self, preview: &[HistoryMessage], live: &[HistoryMessage], limit: usize, retain_context: bool) -> Messages {
        let mut result = merge_direct_history(&self.loaded, preview);
        if retain_context {
            result = merge_direct_history(&result, &self.visible);
        }
        self.candidates.clone_from(&result);
        let mut matches = Vec::new();
        let mut result = filter_history_duplicates(&result, live, retain_context.then_some(&mut matches));
        if limit > 0 && result.len() > limit {
            result.drain(..result.len() - limit);
        }

        self.visible = if retain_context { result.clone() } else { Vec::new() };
        if retain_context {
            retain_history_anchors(&mut self.visible, &matches, &self.loaded);
        }
        result
    }

    /// The newest service key this view knows, consumed and retained rows
    /// included, so that a new send cannot claim an older match; the
    /// default key when none. Port of History::latestServiceKey.
    pub fn latest_service_key(&self) -> TimeUuidKey {
        let mut latest = TimeUuidKey::default();
        for row in self.loaded.iter().chain(&self.visible).chain(&self.candidates) {
            if let Some(key) = service_key(row)
                && (latest == TimeUuidKey::default() || latest < key)
            {
                latest = key;
            }
        }
        latest
    }
}

/// Put `history` (oldest-first) into the session model `current`
/// (newest-first): live rows keep their order and notification rows stay;
/// history rows are marked historical with an integer timestamp. Returns
/// false, leaving `current` untouched, when nothing but the presentation
/// indexes would change, so that the IM window does not replay; otherwise
/// the indexes are renumbered. Port of
/// LLChatServiceHistoryCore::replaceHistory.
pub fn replace_history(current: &mut Messages, history: &[HistoryMessage], direct: bool) -> bool {
    let mut live: Messages = current.iter().filter(|message| !message.is_history).cloned().collect();
    let historical: Messages = history
        .iter()
        .rev()
        .map(|source| HistoryMessage {
            timestamp: Some(source.timestamp.unwrap_or(0)),
            is_history: true,
            is_region_msg: false,
            ..source.clone()
        })
        .collect();
    let merged = if direct {
        interleave_direct_history(historical, live)
    } else {
        live.extend(historical);
        live
    };

    // Live order is kept, so equal positions hold the same live rows: only
    // history values are compared, without their top-level index.
    let unchanged = current.len() == merged.len()
        && current.iter().zip(&merged).all(|(left, right)| {
            left.is_history == right.is_history
                && (!left.is_history
                    || HistoryMessage {
                        index: right.index,
                        ..left.clone()
                    } == *right)
        });
    if unchanged {
        return false;
    }
    *current = merged;
    let count = current.len();
    for (pos, message) in current.iter_mut().enumerate() {
        message.index = Some((count - 1 - pos) as i32);
    }
    true
}
