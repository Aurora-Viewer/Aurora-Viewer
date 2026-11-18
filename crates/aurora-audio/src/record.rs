//! The viewer's own mix, for the Director's film (Aurora's own: the Legacy
//! Viewer records no film). The mixer writes, beside what the device
//! plays, the sum of the channels chosen (the world's sounds, the media, the
//! interface; voice only when asked: it records other people) at the volumes
//! and mutes the user hears. It is handed over in blocks through a bounded
//! queue: the audio callback never waits and allocates nothing. A block the
//! reader did not take in time is lost, and the next block carries how many
//! frames were lost before it, so that the film's sound is given silence of
//! the same length in their place and stays in step with the picture.

use crate::mixer::{BUS_CHANNELS, Channel, ENGINE_CHANNELS, NUM_BUSES};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

/// Stereo frames a block holds (about 21 ms at 48 kHz).
pub const RECORD_BLOCK: usize = 1024;
/// Blocks waiting for the reader: about 2.7 s of sound.
const QUEUE: usize = 128;

/// The frames of a block: interleaved stereo at 48 kHz.
pub type Frames = Box<[f32; RECORD_BLOCK * ENGINE_CHANNELS]>;

/// A block of the mix, and the frames lost just before it.
pub struct Block {
    pub frames: Frames,
    pub lost_before: u64,
}

/// The mixer's side: which buses go in, and the queue it fills.
pub(crate) struct RecordSink {
    buses: [bool; NUM_BUSES],
    /// The reverb's tail goes in with the Sounds or the Ambient channel,
    /// whose sounds feed it.
    pub(crate) wet: bool,
    tx: Sender<Block>,
    /// Blocks made ahead and given back by the reader.
    spare: Receiver<Frames>,
    block: Option<Frames>,
    fill: usize,
    /// Frames lost since the last block sent.
    lost: u64,
}

impl RecordSink {
    pub(crate) fn takes(&self, bus: usize) -> bool {
        self.buses.get(bus).copied().unwrap_or(false)
    }

    /// One stereo frame of the recorded mix.
    pub(crate) fn push(&mut self, l: f32, r: f32) {
        if self.block.is_none() {
            self.block = self.spare.try_recv().ok();
            self.fill = 0;
        }
        let Some(block) = self.block.as_mut() else {
            // no block to write in: this frame is lost
            self.lost += 1;
            return;
        };
        block[2 * self.fill] = l.clamp(-1.0, 1.0);
        block[2 * self.fill + 1] = r.clamp(-1.0, 1.0);
        self.fill += 1;
        if self.fill < RECORD_BLOCK {
            return;
        }
        let Some(frames) = self.block.take() else {
            return;
        };
        let sent = Block {
            frames,
            lost_before: self.lost,
        };
        match self.tx.try_send(sent) {
            Ok(()) => self.lost = 0,
            Err(TrySendError::Full(back) | TrySendError::Disconnected(back)) => {
                // the reader is behind: the block's frames are lost, its
                // memory is filled again
                self.lost += RECORD_BLOCK as u64;
                self.block = Some(back.frames);
                self.fill = 0;
            }
        }
    }
}

/// The reader's side: the blocks of the mix, in order.
pub struct RecordTap {
    rx: Receiver<Block>,
    /// Empty blocks given back to the mixer.
    spare: Sender<Frames>,
}

impl RecordTap {
    /// The next block of the mix, if one is ready.
    pub fn next(&self) -> Option<Block> {
        self.rx.try_recv().ok()
    }

    /// A block read, given back to be filled again.
    pub fn recycle(&self, frames: Frames) {
        let _ = self.spare.try_send(frames);
    }
}

/// A sink for the mixer and its tap, the `channels` taken (the master is
/// not a bus: it scales every channel as the user hears them).
pub(crate) fn pair(channels: &[Channel]) -> (RecordSink, RecordTap) {
    let buses = std::array::from_fn(|i| channels.contains(&BUS_CHANNELS[i]));
    let (tx, rx) = bounded(QUEUE);
    let (spare_tx, spare_rx) = bounded(QUEUE + 2);
    for _ in 0..QUEUE + 2 {
        let _ = spare_tx.try_send(Box::new([0.0; RECORD_BLOCK * ENGINE_CHANNELS]));
    }
    let wet = channels.iter().any(|c| matches!(c, Channel::Sfx | Channel::Ambient));
    (
        RecordSink {
            buses,
            wet,
            tx,
            spare: spare_rx,
            block: None,
            fill: 0,
            lost: 0,
        },
        RecordTap { rx, spare: spare_tx },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_reaches_the_tap_in_blocks_and_lost_frames_are_counted() {
        let (mut sink, tap) = pair(&[Channel::Sfx, Channel::Music]);
        assert!(sink.wet, "the sounds feed the reverb");
        assert!(sink.takes(Channel::Sfx.bus().expect("bus")));
        assert!(!sink.takes(Channel::Voice.bus().expect("bus")), "voice only when asked");
        assert!(!sink.takes(99));
        for f in 0..RECORD_BLOCK + 10 {
            sink.push(f as f32 / 1e4, 2.0);
        }
        let block = tap.next().expect("a block");
        assert_eq!(block.lost_before, 0);
        assert_eq!(block.frames[2], 1.0 / 1e4);
        assert_eq!(block.frames[1], 1.0, "clamped");
        assert!(tap.next().is_none(), "the next block is not full yet");
        tap.recycle(block.frames);
        // the reader away: the queue fills, then blocks are lost rather
        // than the callback waiting
        let more = (QUEUE + 3) * RECORD_BLOCK;
        for _ in 0..more {
            sink.push(0.0, 0.0);
        }
        let mut read = 0;
        while let Some(block) = tap.next() {
            assert_eq!(block.lost_before, 0, "lost after the queued ones");
            read += RECORD_BLOCK;
            tap.recycle(block.frames);
        }
        assert_eq!(read, QUEUE * RECORD_BLOCK);
        // the next block says how much was lost before it
        for _ in 0..RECORD_BLOCK {
            sink.push(0.0, 0.0);
        }
        let next = tap.next().expect("a block");
        let pushed = RECORD_BLOCK + 10 + more + RECORD_BLOCK;
        let accounted = RECORD_BLOCK + read + next.lost_before as usize + RECORD_BLOCK;
        assert!(next.lost_before > 0);
        assert_eq!(pushed - accounted, sink.fill, "only the block being filled is left");
        assert!(!pair(&[Channel::Voice]).0.wet);
    }
}
