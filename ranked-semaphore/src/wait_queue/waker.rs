use std::cell::UnsafeCell;
use std::task::Waker;

const NUM_WAKERS: usize = 32;

/// A thread-safe wrapper around `Waker` that requires external synchronization.
///
/// This wrapper provides safe access to a `Waker` stored in an `UnsafeCell`.
/// All methods require that the caller holds the appropriate lock (wait queue mutex)
/// to ensure thread safety.
pub(crate) struct SafeWakerCell {
    waker: UnsafeCell<Option<Waker>>,
}

impl SafeWakerCell {
    pub(crate) fn new() -> Self {
        Self {
            waker: UnsafeCell::new(None),
        }
    }

    /// Sets the waker for this cell.
    ///
    /// # Safety
    ///
    /// The caller must hold the wait queue mutex to ensure exclusive access.
    /// Multiple threads must not call this method concurrently.
    pub(crate) unsafe fn set_under_lock(&self, waker: Waker) {
        *self.waker.get() = Some(waker);
    }

    /// Takes the waker from this cell, leaving `None` in its place.
    ///
    /// # Safety
    ///
    /// The caller must hold the wait queue mutex to ensure exclusive access.
    /// Multiple threads must not call this method concurrently.
    pub(crate) unsafe fn take_under_lock(&self) -> Option<Waker> {
        (*self.waker.get()).take()
    }

    /// Wakes the task by reference without taking ownership of the waker.
    ///
    /// # Safety
    ///
    /// The caller must hold the wait queue mutex to ensure exclusive access.
    /// Multiple threads must not call this method concurrently.
    pub(crate) unsafe fn wake_by_ref_under_lock(&self) {
        if let Some(waker) = (*self.waker.get()).as_ref() {
            waker.wake_by_ref();
        }
    }

    /// Checks if this cell contains a waker.
    ///
    /// # Safety
    ///
    /// The caller must hold the wait queue mutex to ensure exclusive access.
    /// Multiple threads must not call this method concurrently.
    pub(crate) unsafe fn has_waker_under_lock(&self) -> bool {
        (*self.waker.get()).is_some()
    }
}

// Safety: External mutex protection, Waker is Send + Sync
unsafe impl Send for SafeWakerCell {}
unsafe impl Sync for SafeWakerCell {}

/// A stack-allocated collection of wakers for efficient batch waking.
///
/// This structure allows collecting multiple wakers without heap allocation
/// and waking them all at once, reducing overhead in high-contention scenarios.
pub(crate) struct WakeList {
    wakers: [Option<Waker>; NUM_WAKERS],
    count: usize,
}

impl WakeList {
    /// Creates a new empty wake list.
    pub(crate) fn new() -> Self {
        Self {
            wakers: [const { None }; NUM_WAKERS],
            count: 0,
        }
    }

    pub(crate) fn can_push(&self) -> bool {
        self.count < NUM_WAKERS
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub(crate) fn was_full(&self) -> bool {
        self.count == NUM_WAKERS
    }

    /// Adds a waker to the list.
    ///
    /// # Panics
    ///
    /// Panics if the list is already full (32 wakers). Use [`can_push()`](Self::can_push)
    /// to check available space before calling this method.
    pub(crate) fn push(&mut self, waker: Waker) {
        debug_assert!(self.can_push(), "WakeList is full");
        self.wakers[self.count] = Some(waker);
        self.count += 1;
    }

    /// Adds a waker to the list without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `count < NUM_WAKERS` (32) before calling
    /// this method. Violating this invariant will cause undefined behavior.
    pub(crate) unsafe fn push_unchecked(&mut self, waker: Waker) {
        debug_assert!(self.can_push(), "WakeList is full");
        *self.wakers.get_unchecked_mut(self.count) = Some(waker);
        self.count += 1;
    }

    pub(crate) fn wake_all(&mut self) {
        for i in 0..self.count {
            if let Some(waker) = &self.wakers[i] {
                waker.wake_by_ref();
            }
        }

        for i in 0..self.count {
            self.wakers[i] = None;
        }
        self.count = 0;
    }

    /// Wakes all tasks in the list and clears it, using unchecked indexing.
    ///
    /// # Safety
    ///
    /// This method uses unsafe indexing operations that are valid because
    /// `count` is always maintained to be within valid bounds.
    #[allow(dead_code)]
    pub(crate) unsafe fn wake_all_unchecked(&mut self) {
        for i in 0..self.count {
            if let Some(waker) = self.wakers.get_unchecked_mut(i).take() {
                waker.wake();
            }
        }
        self.count = 0;
    }
}