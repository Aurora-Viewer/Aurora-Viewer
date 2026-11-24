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
//!
//! The simulator's numbers are bounded: an image larger than
//! [`MAX_IMAGE_BYTES`], a packet count that does not fit, or a packet index
//! past that size is ignored, and the packets held before their header are
//! capped for all the images together ([`MAX_EARLY_PACKETS`]), so a broken or
//! hostile simulator cannot make the viewer keep or reserve unbounded memory.
//! Packets that do not add up to the header's size are asked again a few
//! times, waiting longer each time, then the image is given up as missing.
//!
//! `ImageNotInDatabase` marks an image missing only when it comes from the
//! simulator it was asked of and the image is not already complete. A missing
//! image is asked again when the viewer acquires it afresh ([`Want::fresh`]),
//! as `LLVOAvatar::getBakedTextureImage` clears the missing flag
//! (`setIsMissingAsset(false)`) on every `setTETexture`, or when the avatar's
//! region changes.

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
/// Largest image accepted over UDP. A 1024×1024 bake is far below this.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Packets of a [`MAX_IMAGE_BYTES`] image: the 600-byte first packet, then
/// 1000 bytes each.
const MAX_PACKETS: usize = 1 + (MAX_IMAGE_BYTES - FIRST_PACKET_SIZE).div_ceil(MAX_IMG_PACKET_SIZE);
/// Packets held before their image's header, all images together. The
/// Legacy Viewer drops every one of them (`receiveImagePacket` before the
/// header cancels); a reordered packet or two is what this keeps.
pub const MAX_EARLY_PACKETS: usize = 256;
/// Times packets short of the header's size are asked again before the
/// image is given up as missing. The first retry waits [`MIN_REQUEST_TIME`],
/// each next one twice as long.
const MAX_SHORT_RETRIES: u8 = 3;

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
    /// The viewer has just acquired the texture (a new entry, or one nothing
    /// held any more): an earlier `ImageNotInDatabase` is forgotten and the
    /// image is asked again (`setIsMissingAsset(false)` on `setTETexture`).
    pub fresh: bool,
}

/// What a received packet completed.
#[derive(Debug, Clone, PartialEq)]
pub enum Arrival {
    /// Every packet, in index order, cut to the header's size.
    Complete(Vec<u8>),
    /// The packets never added up to the header's size: given up as missing.
    GaveUp,
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
    /// Times the packets came short of the header's size.
    short: u8,
    /// Not asked again before this (after packets that came short).
    not_before: Option<Instant>,
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
    /// `ImageNotInDatabase` stays missing, so a later want does not ask again
    /// unless it is [`Want::fresh`] or names another region. Returns the
    /// images wanted that stay missing, for the viewer to be told again (an
    /// entry that missed the first answer would otherwise wait forever).
    pub fn sync(&mut self, wants: &[Want]) -> Vec<Uuid> {
        for slot in self.slots.values_mut() {
            slot.seen = slot.missing;
        }
        let mut still_missing = Vec::new();
        for w in wants {
            if let Some(slot) = self.slots.get_mut(&w.image) {
                slot.seen = true;
                if slot.missing && (slot.region != w.region || w.fresh) {
                    // Another simulator may have it (the avatar moved, or we
                    // did), or the viewer acquired it again.
                    *slot = Slot::new(w);
                    continue;
                }
                if slot.missing {
                    still_missing.push(w.image);
                    continue;
                }
                if slot.region != w.region {
                    // The avatar's new simulator is asked at once, not at the
                    // next lazy flush, and the old one is told to stop
                    // (an unfinished bake follows its avatar).
                    if !slot.done {
                        if slot.sent_discard.is_some() {
                            self.cancels.push((slot.region, w.image));
                        }
                        // From packet 0: two simulators need not serve the
                        // same bytes for an id (OpenSim sends its MissingImage
                        // under the id asked, J2KImage), so the old one's
                        // packets are not joined to the new one's. The Legacy
                        // Viewer starts over too (LLTextureFetch::createRequest
                        // removes a request whose host changed).
                        *slot = Slot::new(w);
                        continue;
                    }
                    slot.region = w.region;
                    slot.sent_discard = None;
                    slot.not_before = None;
                }
                slot.discard = w.discard;
                slot.priority = w.priority;
                slot.bake = w.bake;
            } else {
                self.slots.insert(w.image, Slot::new(w));
            }
        }
        // A cancel still waiting for an image wanted again is dropped
        // (`LLTextureFetch::addToNetworkQueue` erases it from mCancelQueue),
        // else it would leave after the new request and stop it.
        // A cancel to the region the image is asked of now is meant for that
        // region's earlier request only.
        self.cancels
            .retain(|(region, id)| wants.iter().all(|w| w.image != *id) || self.slots.get(id).is_some_and(|slot| slot.region != *region));
        // A finished transfer is left alone (`removeFromNetworkQueue(worker,
        // false)`); only one still in flight is cancelled.
        let dropped: Vec<(u64, Uuid, bool)> = self
            .slots
            .iter()
            .filter(|(_, slot)| !slot.seen && !slot.missing)
            .map(|(id, slot)| (slot.region, *id, slot.done))
            .collect();
        for (region, id, done) in dropped {
            self.slots.remove(&id);
            if !done {
                self.cancels.push((region, id));
            }
        }
        still_missing
    }

    /// Requests and cancels that are due at `now`. Empty when the last send
    /// was under 100 ms ago, or when nothing is due. The timer advances only
    /// when something is returned, so the first want leaves at once and an
    /// idle fetch does not postpone the next one.
    ///
    /// `has_circuit` says whether a region can be sent to: a slot of a region
    /// without one is left unmarked, so it leaves at once when the circuit
    /// comes, and a cancel to it is dropped (nothing there to stop).
    pub fn take_outbound(&mut self, now: Instant, has_circuit: impl Fn(u64) -> bool) -> Vec<Batch> {
        if self.last_flush.is_some_and(|t| now.saturating_duration_since(t) < REQUEST_DELTA) {
            return Vec::new();
        }
        let mut requests: BTreeMap<u64, Vec<ImageRequest>> = BTreeMap::new();
        for (id, slot) in &mut self.slots {
            if !slot.due(now) || !has_circuit(slot.region) {
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
        for (region, image) in std::mem::take(&mut self.cancels)
            .into_iter()
            .filter(|(region, _)| has_circuit(*region))
        {
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
    /// complete yet.
    pub fn header(&mut self, id: Uuid, region: u64, packets: u16, size: u32, data: Vec<u8>, now: Instant) -> Option<Arrival> {
        let slot = self.slots.get_mut(&id)?;
        // (a late answer of the simulator it was asked of before)
        if slot.region != region || slot.missing || slot.done || slot.header {
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
        if size as usize > MAX_IMAGE_BYTES || usize::from(packets) != packet_count(size as usize) {
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
        assemble(slot, now)
    }

    /// `ImagePacket`. Stored by index even before the header, so a packet
    /// that overtakes `ImageData` is not thrown away, up to
    /// [`MAX_EARLY_PACKETS`] for all the images. A duplicate is ignored.
    pub fn packet(&mut self, id: Uuid, region: u64, num: u16, data: Vec<u8>, now: Instant) -> Option<Arrival> {
        let early = self.early_packets();
        let slot = self.slots.get_mut(&id)?;
        if slot.region != region || slot.missing || slot.done || data.is_empty() {
            return None;
        }
        if slot.header && !packet_fits(num, slot.total, &data) {
            return None;
        }
        if usize::from(num) >= MAX_PACKETS || data.len() > MAX_IMG_PACKET_SIZE {
            return None;
        }
        if slot.parts.contains_key(&num) {
            return None;
        }
        if !slot.header && early >= MAX_EARLY_PACKETS {
            return None;
        }
        slot.parts.insert(num, data);
        slot.sent_at = Some(now);
        assemble(slot, now)
    }

    /// Packets held for images whose header has not arrived.
    fn early_packets(&self) -> usize {
        self.slots.values().filter(|slot| !slot.header).map(|slot| slot.parts.len()).sum()
    }

    /// `ImageNotInDatabase` from `region` (`LLViewerTextureList` sets the
    /// image missing). Sticky: there is no retry timer, only a fresh want.
    /// An image we never asked for is ignored, as the Legacy Viewer marks
    /// only an image `findImage` knows; so is the answer of a simulator the
    /// image is no longer asked of, and one for an image already complete.
    pub fn not_in_database(&mut self, id: Uuid, region: u64) -> bool {
        let Some(slot) = self.slots.get_mut(&id) else {
            return false;
        };
        if slot.region != region || slot.done || slot.missing {
            return false;
        }
        slot.missing = true;
        slot.seen = true;
        slot.parts.clear();
        slot.header = false;
        slot.done = false;
        true
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
            short: 0,
            not_before: None,
        }
    }

    /// Not missing, not already complete, and the discard changed, or the
    /// priority moved enough after a second, or ten seconds have passed.
    /// The first request is due because nothing has been sent.
    fn due(&self, now: Instant) -> bool {
        if self.missing || self.done || self.not_before.is_some_and(|t| now < t) {
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

/// Packets the simulator sends for an image of `size` bytes
/// (`LLImageJ2C` over UDP: 600 bytes, then 1000 each).
fn packet_count(size: usize) -> usize {
    if size <= FIRST_PACKET_SIZE {
        1
    } else {
        1 + (size - FIRST_PACKET_SIZE).div_ceil(MAX_IMG_PACKET_SIZE)
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

fn assemble(slot: &mut Slot, now: Instant) -> Option<Arrival> {
    advance(slot);
    if slot.done || !slot.header || slot.total == 0 || slot.last < i32::from(slot.total) - 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity((slot.size as usize).min(MAX_IMAGE_BYTES));
    for index in 0..slot.total {
        bytes.extend_from_slice(slot.parts.get(&index)?);
    }
    if bytes.len() < slot.size as usize {
        // The packets do not add up to the header's size: ask again from the
        // start, later each time, then give up rather than download it again
        // and again.
        slot.parts.clear();
        slot.header = false;
        slot.last = -1;
        slot.short = slot.short.saturating_add(1);
        if slot.short > MAX_SHORT_RETRIES {
            slot.missing = true;
            slot.seen = true;
            return Some(Arrival::GaveUp);
        }
        slot.sent_discard = None;
        slot.not_before = Some(now + MIN_REQUEST_TIME * (1u32 << (slot.short - 1)));
        return None;
    }
    bytes.truncate(slot.size as usize);
    slot.done = true;
    slot.parts.clear();
    Some(Arrival::Complete(bytes))
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
            fresh: false,
        }
    }

    #[test]
    fn requests_leave_in_batches_of_fifty_with_the_bake_type() {
        let mut fetch = Fetch::default();
        let wants: Vec<Want> = (1..=51).map(want).collect();
        fetch.sync(&wants);
        let t0 = Instant::now();
        let batches = fetch.take_outbound(t0, |_| true);
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
        assert!(fetch.take_outbound(t0 + Duration::from_millis(50), |_| true).is_empty());
    }

    /// A want for a region with no circuit yet is not spent on a send that
    /// cannot leave: it goes out as soon as the circuit is there, not after
    /// SIM_LAZY_FLUSH; a cancel to such a region is dropped.
    #[test]
    fn a_region_without_a_circuit_keeps_its_requests_due() {
        let mut fetch = Fetch::default();
        fetch.sync(&[want(1)]);
        let t0 = Instant::now();
        assert!(fetch.take_outbound(t0, |_| false).is_empty());
        let out = fetch.take_outbound(t0 + REQUEST_DELTA, |region| region == 7);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].items[0].packet, 0);
        // given up while unreachable: no cancel is sent anywhere
        fetch.sync(&[]);
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA * 2, |_| false).is_empty());
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA * 3, |_| true).is_empty());
    }

    #[test]
    fn the_next_request_asks_for_the_packet_after_the_header() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(4);
        fetch.sync(&[want(4)]);
        let t0 = Instant::now();
        let first = fetch.take_outbound(t0, |_| true);
        assert_eq!(first[0].items[0].packet, 0);
        let header = vec![1u8; FIRST_PACKET_SIZE];
        assert!(fetch.header(id, 7, 2, 610, header, t0).is_none());
        let again = fetch.take_outbound(t0 + SIM_LAZY_FLUSH, |_| true);
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
        assert!(fetch.packet(id, 7, 1, tail.clone(), t0).is_none());
        // a duplicate is ignored, not a reason to cancel the transfer
        assert!(fetch.packet(id, 7, 1, vec![9u8; 10], t0).is_none());
        let head = vec![1u8; FIRST_PACKET_SIZE];
        let got = fetch
            .header(id, 7, 2, (FIRST_PACKET_SIZE + tail.len()) as u32, head.clone(), t0)
            .unwrap();
        let mut expect = head;
        expect.extend_from_slice(&tail);
        assert_eq!(got, Arrival::Complete(expect));
    }

    #[test]
    fn image_not_in_database_is_not_asked_again() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(6);
        let w = want(6);
        fetch.sync(std::slice::from_ref(&w));
        let t0 = Instant::now();
        assert_eq!(fetch.take_outbound(t0, |_| true).len(), 1);
        assert!(fetch.not_in_database(id, 7));
        assert!(fetch.is_missing(&id));
        assert!(fetch.take_outbound(t0 + Duration::from_secs(600), |_| true).is_empty());
        // still missing: the viewer is told again, nothing is asked
        assert_eq!(fetch.sync(std::slice::from_ref(&w)), vec![id]);
        assert!(fetch.take_outbound(t0 + Duration::from_secs(601), |_| true).is_empty());
    }

    #[test]
    fn a_dropped_want_is_cancelled_with_discard_level_minus_one() {
        let mut fetch = Fetch::default();
        fetch.sync(&[want(3)]);
        let t0 = Instant::now();
        assert_eq!(fetch.take_outbound(t0, |_| true).len(), 1);
        fetch.sync(&[]);
        let cancel = fetch.take_outbound(t0 + REQUEST_DELTA, |_| true);
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

    #[test]
    fn a_simulator_cannot_make_us_hold_unbounded_bytes() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(10);
        fetch.sync(&[want(10)]);
        let t0 = Instant::now();
        // a packet index past the largest image accepted is not kept
        assert!(fetch.packet(id, 7, u16::MAX, vec![0u8; 1000], t0).is_none());
        assert!(fetch.packet(id, 7, 1, vec![0u8; MAX_IMG_PACKET_SIZE + 1], t0).is_none());
        assert!(fetch.slots[&id].parts.is_empty());
        // a size beyond the limit, or a packet count that does not match it
        let head = vec![1u8; FIRST_PACKET_SIZE];
        assert!(fetch.header(id, 7, 2, u32::MAX, head.clone(), t0).is_none());
        assert!(fetch.header(id, 7, 60_000, 610, head.clone(), t0).is_none());
        assert!(!fetch.slots[&id].header);
        // a sane header is still taken
        assert!(fetch.header(id, 7, 2, 610, head, t0).is_none());
        assert!(fetch.slots[&id].header);
    }

    #[test]
    fn packets_short_of_the_header_size_are_asked_again() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(11);
        fetch.sync(&[want(11)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        // header says 2600 bytes in 3 packets; the last one comes short
        let head = vec![1u8; FIRST_PACKET_SIZE];
        assert!(fetch.header(id, 7, 3, 2600, head, t0).is_none());
        assert!(fetch.slots[&id].header);
        assert!(fetch.packet(id, 7, 1, vec![2u8; 1000], t0).is_none());
        assert!(fetch.packet(id, 7, 2, vec![4u8; 10], t0).is_none());
        // asked again from the start, but not before a second has passed
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA, |_| true).is_empty());
        let again = fetch.take_outbound(t0 + MIN_REQUEST_TIME, |_| true);
        assert_eq!(again[0].items[0].packet, 0, "{again:?}");
    }

    /// Header, middle packet and a short last packet: 2600 bytes announced,
    /// 1610 sent.
    fn send_short(fetch: &mut Fetch, id: Uuid, now: Instant) -> Option<Arrival> {
        assert!(fetch.header(id, 7, 3, 2600, vec![1u8; FIRST_PACKET_SIZE], now).is_none());
        assert!(fetch.packet(id, 7, 1, vec![2u8; 1000], now).is_none());
        fetch.packet(id, 7, 2, vec![4u8; 10], now)
    }

    #[test]
    fn packets_that_keep_coming_short_are_given_up_with_a_growing_wait() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(16);
        fetch.sync(&[want(16)]);
        let mut now = Instant::now();
        assert_eq!(fetch.take_outbound(now, |_| true).len(), 1);
        for wait in [1, 2, 4] {
            assert!(send_short(&mut fetch, id, now).is_none());
            let wait = Duration::from_secs(wait);
            assert!(
                fetch.take_outbound(now + wait - REQUEST_DELTA, |_| true).is_empty(),
                "asked again before {wait:?}"
            );
            now += wait;
            assert_eq!(fetch.take_outbound(now, |_| true).len(), 1, "asked again after {wait:?}");
        }
        assert_eq!(send_short(&mut fetch, id, now), Some(Arrival::GaveUp));
        assert!(fetch.is_missing(&id));
        assert!(fetch.take_outbound(now + SIM_LAZY_FLUSH, |_| true).is_empty());
    }

    #[test]
    fn a_missing_image_acquired_afresh_is_asked_again() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(17);
        fetch.sync(&[want(17)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        assert!(fetch.not_in_database(id, 7));
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA, |_| true).is_empty());
        let fresh = Want { fresh: true, ..want(17) };
        assert!(fetch.sync(std::slice::from_ref(&fresh)).is_empty());
        assert!(!fetch.is_missing(&id));
        let out = fetch.take_outbound(t0 + REQUEST_DELTA * 2, |_| true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].items[0].packet, 0);
    }

    #[test]
    fn image_not_in_database_counts_only_from_the_region_asked_and_before_the_end() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(18);
        fetch.sync(&[want(18)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        // another simulator does not decide for the one asked
        assert!(!fetch.not_in_database(id, 8));
        assert!(!fetch.is_missing(&id));
        // a finished image stays finished
        assert_eq!(
            fetch.header(id, 7, 1, 300, vec![5u8; 300], t0),
            Some(Arrival::Complete(vec![5u8; 300]))
        );
        assert!(!fetch.not_in_database(id, 7));
        assert!(!fetch.is_missing(&id));
    }

    #[test]
    fn a_want_that_moves_region_is_asked_at_once_and_cancelled_at_the_old_one() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(19);
        fetch.sync(&[want(19)]);
        let t0 = Instant::now();
        assert!(fetch.header(id, 7, 2, 610, vec![1u8; FIRST_PACKET_SIZE], t0).is_none());
        assert_eq!(fetch.take_outbound(t0, |_| true).len(), 1);
        let moved = Want { region: 8, ..want(19) };
        fetch.sync(std::slice::from_ref(&moved));
        let out = fetch.take_outbound(t0 + REQUEST_DELTA, |_| true);
        let by_region: Vec<(u64, i8, u32)> = out
            .iter()
            .flat_map(|batch| batch.items.iter().map(|item| (batch.region, item.discard, item.packet)))
            .collect();
        // the new simulator is asked from packet 0: its bytes may differ
        assert!(by_region.contains(&(8, moved.discard, 0)), "{out:?}");
        assert!(by_region.contains(&(7, -1, 0)), "{out:?}");
        assert_eq!(by_region.len(), 2, "{out:?}");
        // the same region again sends no cancel
        fetch.sync(std::slice::from_ref(&moved));
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA * 2, |_| true).is_empty());
        // a late answer of the old simulator is not joined to the new one's
        assert!(fetch.packet(id, 7, 1, vec![2u8; 10], t0).is_none());
        assert!(fetch.slots[&id].parts.is_empty() && !fetch.slots[&id].header);
        assert_eq!(
            fetch.header(id, 8, 1, 300, vec![5u8; 300], t0),
            Some(Arrival::Complete(vec![5u8; 300]))
        );
    }

    #[test]
    fn packets_before_their_header_are_capped_for_all_images_together() {
        let mut fetch = Fetch::default();
        let wants: Vec<Want> = (100..110).map(want).collect();
        fetch.sync(&wants);
        let t0 = Instant::now();
        let mut kept = 0;
        for w in &wants {
            for num in 1..=100u16 {
                fetch.packet(w.image, 7, num, vec![0u8; MAX_IMG_PACKET_SIZE], t0);
            }
            kept += fetch.slots[&w.image].parts.len();
        }
        assert_eq!(kept, MAX_EARLY_PACKETS);
        assert_eq!(fetch.early_packets(), MAX_EARLY_PACKETS);
        // once a header arrives, that image's packets no longer count
        let first = wants[0].image;
        let held = fetch.slots[&first].parts.len();
        assert!(
            fetch
                .header(first, 7, 200, 600 + 199 * 1000, vec![1u8; FIRST_PACKET_SIZE], t0)
                .is_none()
        );
        assert_eq!(fetch.early_packets(), MAX_EARLY_PACKETS - held);
        assert!(fetch.packet(wants[9].image, 7, 101, vec![0u8; MAX_IMG_PACKET_SIZE], t0).is_none());
        assert_eq!(fetch.early_packets(), MAX_EARLY_PACKETS - held + 1);
    }

    #[test]
    fn an_image_wanted_again_is_not_cancelled_after_its_new_request() {
        let mut fetch = Fetch::default();
        fetch.sync(&[want(12)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        fetch.sync(&[]);
        fetch.sync(&[want(12)]);
        let out = fetch.take_outbound(t0 + REQUEST_DELTA, |_| true);
        let items: Vec<&ImageRequest> = out.iter().flat_map(|batch| &batch.items).collect();
        assert_eq!(items.len(), 1, "{out:?}");
        assert_eq!(items[0].discard, 3);
    }

    #[test]
    fn a_finished_image_is_not_cancelled() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(13);
        fetch.sync(&[want(13)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        assert!(fetch.header(id, 7, 1, 300, vec![5u8; 300], t0).is_some());
        fetch.sync(&[]);
        assert!(fetch.take_outbound(t0 + REQUEST_DELTA, |_| true).is_empty());
    }

    #[test]
    fn image_not_in_database_counts_only_for_an_image_we_asked_for() {
        let mut fetch = Fetch::default();
        assert!(!fetch.not_in_database(Uuid::from_u128(14), 7));
        assert!(fetch.slots.is_empty());
    }

    #[test]
    fn a_missing_bake_is_asked_of_the_avatars_new_region() {
        let mut fetch = Fetch::default();
        let id = Uuid::from_u128(15);
        fetch.sync(&[want(15)]);
        let t0 = Instant::now();
        fetch.take_outbound(t0, |_| true);
        assert!(fetch.not_in_database(id, 7));
        let moved = Want { region: 8, ..want(15) };
        fetch.sync(std::slice::from_ref(&moved));
        assert!(!fetch.is_missing(&id));
        let out = fetch.take_outbound(t0 + REQUEST_DELTA, |_| true);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].region, 8);
    }
}
