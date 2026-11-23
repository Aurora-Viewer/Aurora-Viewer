//! Only one optional transparency build at a time. The render thread polls
//! it while drawing the sorted order and installs a complete, matching set.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

#[derive(Default)]
pub(crate) enum PipelineBuild<T> {
    #[default]
    Idle,
    Building {
        samples: u32,
        obsolete: bool,
        done: Receiver<Option<T>>,
    },
    Ready(u32, T),
    Refused(u32),
}

impl<T> PipelineBuild<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(_, set) => Some(set),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Ready(_, set) => Some(set),
            _ => None,
        }
    }

    pub fn disabled(&mut self) {
        match self {
            Self::Building { obsolete, done, .. } => {
                *obsolete = true;
                if !matches!(done.try_recv(), Err(TryRecvError::Empty)) {
                    *self = Self::Idle;
                }
            }
            Self::Refused(_) => *self = Self::Idle,
            _ => {}
        }
    }

    /// A previous set can supply its sample-independent resources to the
    /// new build. A late or rejected result never reaches a render pass.
    pub fn request(&mut self, samples: u32, spawn: impl FnOnce(Option<T>, Sender<Option<T>>) -> std::io::Result<()>) -> bool {
        match self {
            Self::Ready(active, _) if *active == samples => return true,
            Self::Refused(failed) if *failed == samples => return false,
            Self::Building {
                samples: building,
                obsolete,
                done,
            } => {
                *obsolete |= *building != samples;
                let result = match done.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => return false,
                    Err(TryRecvError::Disconnected) => None,
                };
                if !*obsolete {
                    if let Some(set) = result {
                        *self = Self::Ready(samples, set);
                        return true;
                    }
                    log::warn!("oit stochastic: build failed at {samples} scene sample(s); using sorted transparency");
                    *self = Self::Refused(samples);
                    return false;
                }
            }
            _ => {}
        }

        let previous = match std::mem::take(self) {
            Self::Ready(_, set) => Some(set),
            _ => None,
        };
        let (tx, done) = channel();
        *self = match spawn(previous, tx) {
            Ok(()) => Self::Building {
                samples,
                obsolete: false,
                done,
            },
            Err(e) => {
                log::warn!("oit stochastic: could not start the pipeline worker ({e}); using sorted transparency");
                Self::Refused(samples)
            }
        };
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unexpected(_: Option<u32>, _: Sender<Option<u32>>) -> std::io::Result<()> {
        panic!("another build was started")
    }

    #[test]
    fn sorted_frames_continue_until_the_matching_set_arrives() {
        let mut build = PipelineBuild::Idle;
        let mut finish = None;
        assert!(!build.request(1, |previous, tx| {
            assert!(previous.is_none());
            finish = Some(tx);
            Ok(())
        }));
        let first = finish.take().unwrap();
        for _ in 0..8 {
            assert!(!build.request(1, unexpected));
            assert!(build.ready().is_none());
        }
        // Switching AA does not start a competing driver compile. Discard
        // the stale success and request the currently selected count once.
        assert!(!build.request(4, unexpected));
        first.send(Some(1)).unwrap();
        assert!(!build.request(4, |previous, tx| {
            assert!(previous.is_none());
            tx.send(Some(4)).unwrap();
            Ok(())
        }));
        assert!(build.request(4, unexpected));
        assert_eq!(build.ready(), Some(&4));
        assert!(build.request(4, unexpected));
        assert!(!build.request(1, |previous, _| {
            assert_eq!(previous, Some(4));
            Ok(())
        }));
    }

    #[test]
    fn refusal_or_worker_exit_stays_sorted_until_retry() {
        let mut build = PipelineBuild::Idle;
        assert!(!build.request(1, |_, tx| {
            tx.send(None).unwrap();
            Ok(())
        }));
        assert!(!build.request(1, unexpected));
        assert!(!build.request(1, unexpected));
        build.disabled();
        // Disconnected sender (e.g. worker stopped).
        assert!(!build.request(1, |_, _| Ok(())));
        assert!(!build.request(1, unexpected));
        assert!(!build.request(1, unexpected));
        // A different sample count also permits a retry.
        assert!(!build.request(4, |_, _| Err(std::io::Error::other("no worker"))));
        assert!(!build.request(4, unexpected));
        build.disabled();
        assert!(!build.request(4, |_, tx| {
            tx.send(Some(4)).unwrap();
            Ok(())
        }));
        assert!(build.request(4, unexpected));
    }

    #[test]
    fn reenable_after_disable_in_flight_retries_even_the_same_count() {
        let mut build = PipelineBuild::Idle;
        let mut finish = None;
        assert!(!build.request(1, |_, tx| {
            finish = Some(tx);
            Ok(())
        }));
        build.disabled();
        assert!(!build.request(1, unexpected));
        finish.unwrap().send(None).unwrap();
        assert!(!build.request(1, |_, tx| {
            tx.send(Some(1)).unwrap();
            Ok(())
        }));
        assert!(build.request(1, unexpected));
    }
}
