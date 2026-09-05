use atomic_float::AtomicF32;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use std::sync::Arc;
use std::sync::atomic::{self, AtomicU16, AtomicU64};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u16)]
pub enum ControllerState {
    Cancelled,
    Failed,
    Paused,
    Running,
}

#[derive(Debug)]
struct ControllerInner {
    state: AtomicU16,
    progress: AtomicF32,
    notify: Notify,
    // WMDE: counters behind the progress ratio, so the operations window can show a rate, a
    // remaining time and an item count instead of a bare percentage. Display only - nothing
    // synchronises on them, which is why every access is `Relaxed`.
    items_done: AtomicU64,
    items_total: AtomicU64,
    bytes_done: AtomicU64,
    bytes_total: AtomicU64,
}

/// WMDE: what an operation has done so far, as far as it can tell.
///
/// `bytes_total` is zero when the operation cannot know the volume up front - emptying the
/// trash, for instance. Zero means unknown, not empty: an operation with nothing to do never
/// reaches the window at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub items_done: u64,
    pub items_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

impl Counts {
    /// Volume is known and non-empty, so a rate over it says something.
    pub const fn has_bytes(&self) -> bool {
        self.bytes_total > 0
    }
}

#[derive(Debug)]
pub struct Controller {
    primary: bool,
    inner: Arc<ControllerInner>,
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            primary: true,
            inner: Arc::new(ControllerInner {
                state: AtomicU16::new(ControllerState::Running.into()),
                progress: AtomicF32::new(0.0),
                notify: Notify::new(),
                items_done: AtomicU64::new(0),
                items_total: AtomicU64::new(0),
                bytes_done: AtomicU64::new(0),
                bytes_total: AtomicU64::new(0),
            }),
        }
    }
}

impl Controller {
    pub async fn check(&self) -> Result<(), ControllerState> {
        loop {
            match self.state() {
                ControllerState::Cancelled => return Err(ControllerState::Cancelled),
                ControllerState::Failed => return Err(ControllerState::Failed),
                ControllerState::Paused => (),
                ControllerState::Running => return Ok(()),
            }

            self.inner.notify.notified().await;
        }
    }

    pub fn progress(&self) -> f32 {
        self.inner.progress.load(atomic::Ordering::Relaxed)
    }

    pub fn set_progress(&self, progress: f32) {
        self.inner
            .progress
            .swap(progress, atomic::Ordering::Relaxed);
    }

    /// WMDE: how much there is to do. Called once, before the work starts; `bytes` is zero
    /// when the operation cannot weigh itself in advance.
    pub fn set_totals(&self, items: u64, bytes: u64) {
        self.inner
            .items_total
            .store(items, atomic::Ordering::Relaxed);
        self.inner
            .bytes_total
            .store(bytes, atomic::Ordering::Relaxed);
    }

    /// WMDE: how much is done. Called from the same throttled callback as `set_progress`.
    pub fn set_done(&self, items: u64, bytes: u64) {
        self.inner
            .items_done
            .store(items, atomic::Ordering::Relaxed);
        self.inner
            .bytes_done
            .store(bytes, atomic::Ordering::Relaxed);
    }

    /// WMDE: for operations counted in items rather than bytes - emptying the trash, setting
    /// permissions, restoring. Sets the ratio and the counters from the same pair, so the
    /// window cannot show "3 of 10" next to a bar at half.
    pub fn set_items(&self, done: u64, total: u64) {
        self.set_totals(total, 0);
        self.set_done(done, 0);
        self.set_progress(if total == 0 {
            1.0
        } else {
            done as f32 / total as f32
        });
    }

    /// One snapshot of all four counters.
    ///
    /// The loads are `Relaxed` and independent, so in principle a total can come from one tick
    /// and a done from the next. Totals are written once, before the work starts, and never
    /// move again, which leaves a single frame at the very beginning where a total can still
    /// be zero - and a zero total already means "unknown", so nothing is drawn from it. The
    /// clamp covers the other direction: a file that grew between the survey and the copy.
    pub fn counts(&self) -> Counts {
        let items_total = self.inner.items_total.load(atomic::Ordering::Relaxed);
        let bytes_total = self.inner.bytes_total.load(atomic::Ordering::Relaxed);
        let items_done = self.inner.items_done.load(atomic::Ordering::Relaxed);
        let bytes_done = self.inner.bytes_done.load(atomic::Ordering::Relaxed);
        Counts {
            items_done: if items_total > 0 {
                items_done.min(items_total)
            } else {
                items_done
            },
            items_total,
            bytes_done: if bytes_total > 0 {
                bytes_done.min(bytes_total)
            } else {
                bytes_done
            },
            bytes_total,
        }
    }

    pub fn state(&self) -> ControllerState {
        ControllerState::try_from(self.inner.state.load(atomic::Ordering::Relaxed))
            .unwrap_or(ControllerState::Failed)
    }

    pub fn set_state(&self, state: ControllerState) {
        self.inner
            .state
            .store(state.into(), atomic::Ordering::Relaxed);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self.state(), ControllerState::Cancelled)
    }

    pub fn cancel(&self) {
        self.set_state(ControllerState::Cancelled);
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.state(), ControllerState::Failed)
    }

    pub fn is_paused(&self) -> bool {
        matches!(self.state(), ControllerState::Paused)
    }

    pub fn pause(&self) {
        self.set_state(ControllerState::Paused);
    }

    /// Returns when the state is paused.
    ///
    /// Use this to pause futures.
    pub async fn until_paused(&self) {
        loop {
            if matches!(self.state(), ControllerState::Paused) {
                return;
            }

            self.inner.notify.notified().await;
        }
    }

    /// Returns when state is neither paused, cancelled, nor failed.
    ///
    /// Use this to resume futures.
    pub async fn until_unpaused(&self) {
        loop {
            if !matches!(
                self.state(),
                ControllerState::Paused | ControllerState::Cancelled | ControllerState::Failed
            ) {
                return;
            }

            self.inner.notify.notified().await;
        }
    }

    pub fn unpause(&self) {
        if !self.is_cancelled() | !self.is_failed() {
            self.set_state(ControllerState::Running);
        }
    }
}

impl Clone for Controller {
    fn clone(&self) -> Self {
        Self {
            primary: false,
            inner: self.inner.clone(),
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        // Cancel operations if primary controller is dropped and controller is still running
        if self.primary && self.state() != ControllerState::Failed {
            self.cancel();
        }
    }
}
