//! UDP fetch of a bake the simulator keeps itself.
//!
//! `LLTextureFetch::sendRequestListToSimulators`, `receiveImageHeader`,
//! `receiveImagePacket` and `LLTextureFetchWorker::insertPacket`
//! (`indra/newview/lltexturefetch.cpp`). When the login has no appearance
//! service, `LLVOAvatar::getImage` asks the avatar's object host with
//! `RequestImage` (`FTT_HOST_BAKE`, `mCanUseHTTP = false` in
//! `LLViewerFetchedTexture`) instead of HTTP. OpenSim stores an uploaded
//! bake only in that simulator's local cache, so an external asset server
//! answers 404.
//!
//! Visible difference from the Legacy Viewer, kept on purpose: a packet that
//! arrives before its header is stored and used, and a duplicate packet or
//! a duplicate header is ignored. The Legacy Viewer cancels the transfer
//! (`mCancelQueue`) in those cases and asks again. Cancelling drops bytes
//! that are already arriving. A transfer the viewer no longer wants is
//! still cancelled with `DiscardLevel` -1.

use aurora_msg::msgs::{self, RequestImage};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Images in one `RequestImage` (`IMAGES_PER_REQUEST`).
const IMAGES_PER_REQUEST: usize = 50;
/// How often a simulator is asked (`REQUEST_DELTA_TIME`, 10 Hz).
const REQUEST_DELTA: Duration = Duration::from_millis(100);
/// Priority must move by this much, and a second must pass, before another
/// request (`MIN_DELTA_PRIORITY`, `MIN_REQUEST_TIME`).
const MIN_DELTA_PRIORITY: f32 = 1000.0;
const MIN_REQUEST_TIME: Duration = Duration::from_secs(1);
/// A request is repeated at least this often (`SIM_LAZY_FLUSH_TIMEOUT`).
const SIM_LAZY_FLUSH: Duration = Duration::from_secs(10);
/// Header data, and every packet that is not the first or the last
/// (`FIRST_PACKET_SIZE`, `MAX_IMG_PACKET_SIZE` in `llimage.h`).
const FIRST_PACKET_SIZE: usize = 600;
const MAX_IMG_PACKET_SIZE: usize = 1000;

/// `IMG_TYPE` (`llimage.h`). A host bake is `TYPE_AVATAR_BAKE`
/// (`LLTextureFetchWorker::mType` when the host is set).
pub const TYPE_NORMAL: u8 = 0;
pub const TYPE_AVATAR_BAKE: u8 = 1;

/// One image the viewer wants from a simulator.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub region: u64,
    pub image: Uuid,
    pub discard: i8,
    pub priority: f32,
    pub bake: bool,
}

/// One `RequestImage` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub image: Uuid,
    pub discard: i8,
    pub priority: f32,
    pub packet: u32,
    pub type_: u8,
}

/// One `RequestImage` message for a single simulator.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub region: u64,
    pub items: Vec<ImageRequest>,
}

#[derive(Debug)]
struct Slot {
    region: u64,
    discard: i8,
    priority: f32,
    bake: bool,
    /// Present in the latest [`Fetch::sync`]. A missing image stays.
    seen: bool,
    /// `ImageNotInDatabase`: never asked again.
    missing: bool,
    header: bool,
    total: u16,
    size: u32,
    /// Highest contiguous packet index stored, or -1 when packet 0 is absent
    /// (`mLastPacket` starts at -1, so the first request asks for packet 0).
    last: i32,
    /// Packet index to bytes. Kept even when the header has not arrived.
    parts: std::collections::HashMap<u16, Vec<u8>>,
    done: bool,
    sent_discard: Option<i8>,
    sent_priority: f32,
    sent_at: Option<Instant>,
}

/// Images in flight over UDP, and the requests waiting to leave.
#[derive(Debug, Default)]
pub struct Fetch {
    slots: std::collections::HashMap<Uuid, Slot>,
    cancels: Vec<(u64, Uuid)>,
    last_flush: Option<Instant>,
}

impl Fetch {
    /// Replace the set of images we want. One we drop is cancelled.
    /// `ImageNotInDatabase` stays missing, so a later want does not ask again.
    pub fn sync(&mut self, wants: &[Want]) {
        for slot in self.slots.values_mut() {
            slot.seen = slot.missing;
        }
        for w in wants {
            if let Some(slot) = self.slots.get_mut(&w.image) {
                slot.seen = true;
                if slot.missing {
                    continue;
                }
                slot.region = w.region;
                slot.discard = w.discard;
                slot.priority = w.priority;
                slot.bake = w.bake;
            } else {
                self.slots.insert(w.image, Slot::new(w));
            }
        }
        let cancel: Vec<(u64, Uuid)> = self
            .slots
            .iter()
            .filter(|(_, slot)| !slot.seen && !slot.missing)
            .map(|(id, slot)| (slot.region, *id))
            .collect();
        for (_, id) in &cancel {
            self.slots.remove(id);
        }
        self.cancels.extend(cancel);
    }

    /// Requests and cancels that are due at `now`. Empty when the last send
    /// was under 100 ms ago, or when nothing is due. The timer advances only
    /// when something is returned, so the first want leaves at once and an
    /// idle fetch does not postpone the next one.
    pub fn take_outbound(&mut self, now: Instant) -> Vec<Batch> {
        if self.last_flush.is_some_and(|t| now.saturating_duration_since(t) < REQUEST_DELTA) {
            return Vec::new();
        }
        let mut requests: BTreeMap<u64, Vec<ImageRequest>> = BTreeMap::new();
        for (id, slot) in &mut self.slots {
            if !slot.due(now) {
                continue;
            }
            let item = ImageRequest {
                image: *id,
                discard: slot.discard,
                priority: slot.priority,
                packet: (slot.last + 1) as u32,
                type_: if slot.bake { TYPE_AVATAR_BAKE } else { TYPE_NORMAL },
            };
            slot.sent_discard = Some(slot.discard);
            slot.sent_priority = slot.priority;
            slot.sent_at = Some(now);
            requests.entry(slot.region).or_default().push(item);
        }
        let mut cancel_by: BTreeMap<u64, Vec<ImageRequest>> = BTreeMap::new();
        for (region, image) in std::mem::take(&mut self.cancels) {
            cancel_by.entry(region).or_default().push(ImageRequest {
                image,
                discard: -1,
                priority: 0.0,
                packet: 0,
                type_: TYPE_NORMAL,
            });
        }
        let mut batches = Vec::new();
        for (region, mut items) in requests {
            items.sort_by_key(|item| item.image);
            push_chunks(region, items, &mut batches);
        }
        for (region, mut items) in cancel_by {
            items.sort_by_key(|item| item.image);
            push_chunks(region, items, &mut batches);
        }
        if !batches.is_empty() {
            self.last_flush = Some(now);
        }
        batches
    }

    /// `ImageData`: packet 0. `None` when it is ignored or the file is not
    /// complete yet. The bytes are every packet in index order.
    pub fn header(&mut self, id: Uuid, packets: u16, size: u32, data: Vec<u8>, now: Instant) -> Option<Vec<u8>> {
        let slot = self.slots.get_mut(&id)?;
        if slot.missing || slot.done || slot.header {
            return None;
        }
        // receiveImageHeader: empty data, and data that is neither the first
        // packet's 600 bytes nor the whole file (a single-packet image).
        if data.is_empty() || size == 0 || packets == 0 {
            return None;
        }
        if data.len() != FIRST_PACKET_SIZE && data.len() != size as usize {
            return None;
        }
        slot.header = true;
        slot.total = packets;
        slot.size = size;
        slot.parts.insert(0, data);
        let total = packets;
        slot.parts.retain(|&index, bytes| packet_fits(index, total, bytes));
        // insertPacket resets the request timer as the bytes arrive.
        slot.sent_at = Some(now);
        assemble(slot)
    }

    /// `ImagePacket`. Stored by index even before the header, so a packet
    /// that overtakes `ImageData` is not thrown away. A duplicate is ignored.
    pub fn packet(&mut self, id: Uuid, num: u16, data: Vec<u8>, now: Instant) -> Option<Vec<u8>> {
        let slot = self.slots.get_mut(&id)?;
        if slot.missing || slot.done || data.is_empty() {
            return None;
        }
        if slot.header && !packet_fits(num, slot.total, &data) {
            return None;
        }
        if slot.parts.contains_key(&num) {
            return None;
        }
        slot.parts.insert(num, data);
        slot.sent_at = Some(now);
        assemble(slot)
    }

    /// `ImageNotInDatabase` (`LLViewerTextureList` sets the image missing).
    /// Sticky: there is no retry timer.
    pub fn not_in_database(&mut self, id: Uuid) {
        let slot = self.slots.entry(id).or_insert_with(|| {
            Slot::new(&Want {
                region: 0,
                image: id,
                discard: 0,
                priority: 0.0,
                bake: false,
            })
        });
        slot.missing = true;
        slot.seen = true;
        slot.parts.clear();
        slot.header = false;
        slot.done = false;
    }

    pub fn is_missing(&self, id: &Uuid) -> bool {
        self.slots.get(id).is_some_and(|slot| slot.missing)
    }
}

impl Slot {
    fn new(w: &Want) -> Self {
        Self {
            region: w.region,
            discard: w.discard,
            priority: w.priority,
            bake: w.bake,
            seen: true,
            missing: false,
            header: false,
            total: 0,
            size: 0,
            last: -1,
            parts: std::collections::HashMap::new(),
            done: false,
            sent_discard: None,
            sent_priority: 0.0,
            sent_at: None,
        }
    }

    /// Not missing, not already complete, and the discard changed, or the
    /// priority moved enough after a second, or ten seconds have passed.
    /// The first request is due because nothing has been sent.
    fn due(&self, now: Instant) -> bool {
        if self.missing || self.done {
            return false;
        }
        if self.header && self.total > 0 && self.last >= i32::from(self.total) - 1 {
            return false;
        }
        match (self.sent_discard, self.sent_at) {
            (None, _) => true,
            (Some(sent), Some(at)) => {
                let elapsed = now.saturating_duration_since(at);
                sent != self.discard
                    || ((self.priority - self.sent_priority).abs() > MIN_DELTA_PRIORITY && elapsed >= MIN_REQUEST_TIME)
                    || elapsed >= SIM_LAZY_FLUSH
            }
            (Some(_), None) => true,
        }
    }
}

/// Index 0 is the header. A middle packet is exactly 1000 bytes. The last
/// packet may be short. An index past the header's count does not fit.
fn packet_fits(index: u16, total: u16, data: &[u8]) -> bool {
    if total == 0 || index >= total || data.is_empty() {
        return false;
    }
    let last = total - 1;
    if index > 0 && index < last {
        data.len() == MAX_IMG_PACKET_SIZE
    } else {
        true
    }
}

fn advance(slot: &mut Slot) {
    if !slot.header || slot.total == 0 {
        return;
    }
    while slot.last + 1 < i32::from(slot.total) && slot.parts.contains_key(&((slot.last + 1) as u16)) {
        slot.last += 1;
    }
}

fn assemble(slot: &mut Slot) -> Option<Vec<u8>> {
    advance(slot);
    if slot.done || !slot.header || slot.total == 0 || slot.last < i32::from(slot.total) - 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(slot.size as usize);
    for index in 0..slot.total {
        bytes.extend_from_slice(slot.parts.get(&index)?);
    }
    slot.done = true;
    slot.parts.clear();
    Some(bytes)
}

fn push_chunks(region: u64, items: Vec<ImageRequest>, out: &mut Vec<Batch>) {
    let mut items = items;
    while !items.is_empty() {
        let end = items.len().min(IMAGES_PER_REQUEST);
        let chunk: Vec<ImageRequest> = items.drain(..end).collect();
        out.push(Batch { region, items: chunk });
    }
}

/// `RequestImage` for `items` (at most 50; the caller splits).
pub fn request_message(agent: Uuid, session: Uuid, items: &[ImageRequest]) -> RequestImage {
    let mut message = RequestImage::default();
    message.agent_data.agent_id = agent;
    message.agent_data.session_id = session;
    message.request_image = items
        .iter()
        .map(|item| msgs::request_image::RequestImage {
            image: item.image,
            discard_level: item.discard,
            download_priority: item.priority,
            packet: item.packet,
            type_: item.type_,
        })
        .collect();
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::packet::{build_packet, parse_packet};

    fn want(n: u128) -> Want {
        Want {
            region: 7,
            image: Uuid::from_u128(n),
            discard: 3,
            priority: 12.5,
            bake: true,
        }
    }

    #[test]
    fn requests_leave_in_batches_of_fifty_with_the_bake_type() {
        let mut fetch = Fetch::default();
        let wants: Vec<Want> = (1..=51).map(want).collect();
        fetch.sync(&wants);
        let t0 = Instant::now();
        let batches = fetch.take_outbound(t0);
        assert_eq!(batches.len(), 2, "{batches:?}");
        assert_eq!(batches[0].items.len(), 50);
        assert_eq!(batches[1].items.len(), 1);
        assert!(batches.iter().all(|batch| batch.region == 7));
        for item in batches.iter().flat_map(|batch| &batch.items) {
            assert_eq!(item.packet, 0);
            assert_eq!(item.type_, TYPE_AVATAR_BAKE);
            assert_eq!(item.discard, 3);
            assert_eq!(item.priority, 12.5);
        }
        let agent = Uuid::from_u128(9);
        let session = Uuid::from_u128(8);
        let message = request_message(agent, session, &batches[0].items);
        assert_eq!(message.agent_data.agent_id, agent);
        assert_eq!(message.agent_data.session_id, session);
        assert_eq!(message.request_image.len(), 50);
        assert_eq!(message.request_image[0].discard_level, 3);
        assert_eq!(message.request_image[0].download_priority, 12.5);
        assert_eq!(message.request_image[0].packet, 0);
        assert_eq!(message.request_image[0].type_, TYPE_AVATAR_BAKE);
        let mut buf = Vec::new();
        build_packet(&message, 1, true, false, &[], &mut buf);
        let back: RequestImage = parse_packet(&buf).unwrap().decode().unwrap();
        assert_eq!(back, message);
        // 10 Hz: nothing else leaves in the same tenth of a second.
        assert!(fetch.take_outbound(t0 + Duration::from_millis(50)).is_empty());
    }

    #[test]
    fn the_next_request_asks_for_the_packet_after_the_header() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(4);
        fetch.sync(&[want(4)]);
        let t0 = Instant::now();
        let first = fetch.take_outbound(t0);
        assert_eq!(first[0].items[0].packet, 0);
        let header = vec![1u8; FIRST_PACKET_SIZE];
        assert!(fetch.header(id, 2, 610, header, t0).is_none());
        let again = fetch.take_outbound(t0 + SIM_LAZY_FLUSH);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].items[0].packet, 1);
        assert_eq!(again[0].items[0].type_, TYPE_AVATAR_BAKE);
    }

    #[test]
    fn out_of_order_packets_reassemble_in_index_order() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(5);
        fetch.sync(&[want(5)]);
        let t0 = Instant::now();
        let tail = vec![2u8; 10];
        assert!(fetch.packet(id, 1, tail.clone(), t0).is_none());
        // a duplicate is ignored, not a reason to cancel the transfer
        assert!(fetch.packet(id, 1, vec![9u8; 10], t0).is_none());
        let head = vec![1u8; FIRST_PACKET_SIZE];
        let got = fetch
            .header(id, 2, (FIRST_PACKET_SIZE + tail.len()) as u32, head.clone(), t0)
            .unwrap();
        let mut expect = head;
        expect.extend_from_slice(&tail);
        assert_eq!(got, expect);
    }

    #[test]
    fn image_not_in_database_is_not_asked_again() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(6);
        let w = want(6);
        fetch.sync(std::slice::from_ref(&w));
        let t0 = Instant::now();
        assert_eq!(fetch.take_outbound(t0).len(), 1);
        fetch.not_in_database(id);
        assert!(fetch.is_missing(&id));
        assert!(fetch.take_outbound(t0 + Duration::from_secs(600)).is_empty());
        fetch.sync(std::slice::from_ref(&w));
        assert!(fetch.take_outbound(t0 + Duration::from_secs(601)).is_empty());
    }

    #[test]
    fn a_dropped_want_is_cancelled_with_discard_level_minus_one() {
        let mut fetch = Fetch::default();
        fetch.sync(&[want(3)]);
        let t0 = Instant::now();
        assert_eq!(fetch.take_outbound(t0).len(), 1);
        fetch.sync(&[]);
        let cancel = fetch.take_outbound(t0 + REQUEST_DELTA);
        assert_eq!(cancel.len(), 1);
        assert_eq!(
            cancel[0].items,
            vec![ImageRequest {
                image: Uuid::from_u128(3),
                discard: -1,
                priority: 0.0,
                packet: 0,
                type_: TYPE_NORMAL,
            }]
        );
    }
}
