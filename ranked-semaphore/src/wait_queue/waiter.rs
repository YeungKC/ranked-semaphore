use crate::wait_queue::waker::SafeWakerCell;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::Waker;

// Waiter state constants
const WAITING: usize = 0;
const NOTIFIED: usize = 1;
const CANCELLED: usize = 2;

/// State information for a task waiting to acquire permits.
///
/// This structure tracks the state of a waiting task, including the number
/// of permits needed, priority, and current state (waiting/notified/cancelled).
/// The layout is optimized for cache efficiency.
#[repr(C)]
pub(crate) struct WaiterState {
    state: AtomicUsize,
    pub(crate) permits_needed: usize,
    waker: SafeWakerCell,
    pub(crate) priority: isize,
}

// Safety: Atomics + mutex-protected waker
unsafe impl Send for WaiterState {}
unsafe impl Sync for WaiterState {}

/// A handle to a waiter in the wait queue.
///
/// This handle provides access to the waiter's state and allows
/// for cancellation and notification operations.
pub(crate) struct WaiterHandle {
    pub(crate) state: Arc<WaiterState>,
    _weak_ref: Weak<WaiterState>,
}

impl WaiterState {
    pub(crate) fn new(permits_needed: usize, priority: isize) -> Self {
        Self {
            state: AtomicUsize::new(WAITING),
            permits_needed,
            priority,
            waker: SafeWakerCell::new(),
        }
    }

    pub(crate) fn cancel(&self) {
        // Try to cancel if it's waiting
        if self
            .state
            .compare_exchange(WAITING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // Safety: This is called during semaphore close, where we have exclusive access
            // to the wait queue and no other threads can be accessing wakers concurrently.
            unsafe {
                self.waker.wake_by_ref_under_lock();
            }
            return;
        }

        // Also try to cancel if it's been notified but not yet processed
        if self
            .state
            .compare_exchange(NOTIFIED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // Safety: This is called during semaphore close, where we have exclusive access
            // to the wait queue and no other threads can be accessing wakers concurrently.
            unsafe {
                self.waker.wake_by_ref_under_lock();
            }
        }
    }

    pub(crate) fn is_notified(&self) -> bool {
        self.state.load(Ordering::Relaxed) == NOTIFIED
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Relaxed) == CANCELLED
    }

    pub(crate) fn is_waiting(&self) -> bool {
        self.state.load(Ordering::Relaxed) == WAITING
    }

    /// Sets the waker for this waiter.
    ///
    /// # Safety
    ///
    /// This method should only be called from `Future::poll` before the waiter
    /// enters the shared queue. The caller must ensure exclusive access to the waker.
    pub(crate) fn set_waker(&self, waker: Waker) {
        unsafe {
            self.waker.set_under_lock(waker);
        }
    }

    /// Takes the waker from this waiter, leaving `None` in its place.
    ///
    /// # Safety
    ///
    /// The caller must hold the wait queue mutex to ensure exclusive access
    /// to the waiter's state and prevent concurrent access to the waker.
    pub(crate) unsafe fn take_waker_under_lock(&self) -> Option<Waker> {
        self.waker.take_under_lock()
    }

    pub(crate) fn try_notify(&self) -> bool {
        self.state
            .compare_exchange(WAITING, NOTIFIED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl WaiterHandle {
    pub(crate) fn new(permits_needed: usize, priority: isize) -> Self {
        let waiter_state = Arc::new(WaiterState::new(permits_needed, priority));
        let weak_ref = Arc::downgrade(&waiter_state);

        Self {
            state: waiter_state,
            _weak_ref: weak_ref,
        }
    }
}

impl fmt::Debug for WaiterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Safety: This is only used for debugging and we're just checking if the option is Some
        let has_waker = unsafe { self.waker.has_waker_under_lock() };
        f.debug_struct("WaiterState")
            .field("permits_needed", &self.permits_needed)
            .field("priority", &self.priority)
            .field("state", &self.state.load(Ordering::Relaxed))
            .field("has_waker", &has_waker)
            .finish()
    }
}
