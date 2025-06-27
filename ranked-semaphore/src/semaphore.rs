use crate::config::{PriorityConfig, QueueStrategy};
use crate::error::{AcquireError, TryAcquireError};
use crate::wait_queue::{WaitQueue, WaiterHandle};

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// High-performance priority semaphore
#[derive(Debug)]
pub struct RankedSemaphore {
    /// Combined permit count and status flags using bit operations
    /// Bit layout: [permit_count << 1 | closed_flag]
    /// This optimization reduces atomic operations by 50% compared to separate fields
    permits: AtomicUsize,
    /// Wait queue (used only when needed)
    waiters: Mutex<WaitQueue>,
}

/// Semaphore permit (borrowed version)
pub struct RankedSemaphorePermit<'a> {
    sem: &'a RankedSemaphore,
    permits: u32,
}

/// Semaphore permit (owned version)
pub struct OwnedRankedSemaphorePermit {
    sem: Arc<RankedSemaphore>,
    permits: u32,
}

/// Acquire operation Future
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Acquire<'a> {
    semaphore: &'a RankedSemaphore,
    permits_needed: usize,
    priority: isize,
    waiter_handle: Option<WaiterHandle>,
}

/// Owned acquire operation Future
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct AcquireOwned {
    semaphore: Arc<RankedSemaphore>,
    permits_needed: usize,
    priority: isize,
    waiter_handle: Option<WaiterHandle>,
}

impl RankedSemaphore {
    /// Maximum permits (reserve 3 bits for flags, same as tokio)
    pub const MAX_PERMITS: usize = usize::MAX >> 3;

    /// Bit flag constants for state encoding (aligned with tokio)
    const CLOSED: usize = 1;
    const PERMIT_SHIFT: usize = 1;

    /// Create FIFO semaphore
    pub fn new_fifo(permits: usize) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self::new(permits, QueueStrategy::Fifo)
    }

    /// Create LIFO semaphore
    pub fn new_lifo(permits: usize) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self::new(permits, QueueStrategy::Lifo)
    }

    /// Create semaphore with specified strategy
    pub fn new(permits: usize, default_strategy: QueueStrategy) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        let config = PriorityConfig::new().default_strategy(default_strategy);
        Self::new_with_config(permits, config)
    }

    /// Create semaphore with custom configuration
    pub fn new_with_config(permits: usize, config: PriorityConfig) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self {
            permits: AtomicUsize::new(permits << Self::PERMIT_SHIFT),
            waiters: Mutex::new(WaitQueue::new(config)),
        }
    }

    /// Get current available permits
    pub fn available_permits(&self) -> usize {
        self.permits.load(Ordering::Acquire) >> Self::PERMIT_SHIFT
    }

    /// Check if semaphore is closed
    pub fn is_closed(&self) -> bool {
        self.permits.load(Ordering::Acquire) & Self::CLOSED == Self::CLOSED
    }

    /// Add permits and notify waiters if any.
    pub fn add_permits(&self, added: usize) {
        if added == 0 {
            return;
        }

        // Assign permits to the wait queue, following tokio's approach
        self.add_permits_locked(added, self.waiters.lock().unwrap());
    }

    /// Release `rem` permits to the semaphore's wait list.
    ///
    /// If `rem` exceeds the number of permits needed by the wait list, the
    /// remainder are assigned back to the semaphore.
    ///
    /// This implementation follows tokio's pattern of processing waiters in batches
    /// to minimize thundering herd effects and reduce lock contention.
    fn add_permits_locked(
        &self,
        mut rem: usize,
        waiters: std::sync::MutexGuard<'_, crate::wait_queue::WaitQueue>,
    ) {
        let mut lock = Some(waiters);

        // Process waiters in batches like tokio to prevent thundering herd
        while rem > 0 {
            let mut waiters = lock.take().unwrap_or_else(|| self.waiters.lock().unwrap());

            // Check if queue is empty
            if waiters.is_empty() {
                drop(waiters);
                break;
            }

            // Select waiters to notify in this batch
            let (wake_list, permits_assigned) = waiters.select_waiters_to_notify(rem);
            rem -= permits_assigned;

            // If we couldn't assign any permits, we're done
            if permits_assigned == 0 || wake_list.is_empty() {
                drop(waiters);
                break;
            }

            // Drop the lock before waking to reduce lock contention
            drop(waiters);

            // Wake this batch of waiters
            let mut wake_list = wake_list;
            wake_list.wake_all();

            // If WakeList was not at capacity, we processed all available waiters
            if !wake_list.was_full() {
                break;
            }
        }

        // Add any remaining permits back to the semaphore
        if rem > 0 {
            let prev = self
                .permits
                .fetch_add(rem << Self::PERMIT_SHIFT, Ordering::Release);
            let prev_permits = prev >> Self::PERMIT_SHIFT;

            // Check for overflow after the operation
            if prev_permits + rem > Self::MAX_PERMITS {
                panic!(
                    "number of added permits ({}) would overflow MAX_PERMITS ({})",
                    rem,
                    Self::MAX_PERMITS
                );
            }
        }
    }

    /// Close semaphore and cancel all pending waiters.
    pub fn close(&self) {
        self.permits.fetch_or(Self::CLOSED, Ordering::Release);

        let mut waiters = self.waiters.lock().unwrap();
        waiters.close();
    }

    // === Acquire methods ===

    /// Acquire with default priority
    pub fn acquire(&self) -> Acquire<'_> {
        self.acquire_many_with_priority(0, 1)
    }

    /// Acquire with specified priority
    pub fn acquire_with_priority(&self, priority: isize) -> Acquire<'_> {
        self.acquire_many_with_priority(priority, 1)
    }

    /// Acquire many permits (default priority)
    pub fn acquire_many(&self, n: u32) -> Acquire<'_> {
        self.acquire_many_with_priority(0, n)
    }

    /// Acquire many permits with priority
    pub fn acquire_many_with_priority(&self, priority: isize, n: u32) -> Acquire<'_> {
        Acquire {
            semaphore: self,
            permits_needed: n as usize,
            priority,
            waiter_handle: None,
        }
    }

    // === Non-blocking acquire methods ===

    /// Try acquire single permit (non-blocking)
    pub fn try_acquire(&self) -> Result<RankedSemaphorePermit<'_>, TryAcquireError> {
        let mut curr = self.permits.load(Ordering::Acquire);
        loop {
            // Check if semaphore is closed
            if curr & Self::CLOSED == Self::CLOSED {
                return Err(TryAcquireError::Closed);
            }

            // Check if enough permits are available (need at least 1 << PERMIT_SHIFT)
            if curr < (1 << Self::PERMIT_SHIFT) {
                return Err(TryAcquireError::NoPermits);
            }

            let next = curr - (1 << Self::PERMIT_SHIFT);
            match self.permits.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(RankedSemaphorePermit {
                        sem: self,
                        permits: 1,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    /// Try acquire many permits (non-blocking)
    pub fn try_acquire_many(&self, n: u32) -> Result<RankedSemaphorePermit<'_>, TryAcquireError> {
        if n == 0 {
            return Ok(RankedSemaphorePermit {
                sem: self,
                permits: 0,
            });
        }

        if n as usize > Self::MAX_PERMITS {
            panic!("try_acquire_many: n exceeds MAX_PERMITS");
        }

        let n_shifted = (n as usize) << Self::PERMIT_SHIFT;
        let mut curr = self.permits.load(Ordering::Acquire);
        loop {
            // Check if semaphore is closed
            if curr & Self::CLOSED == Self::CLOSED {
                return Err(TryAcquireError::Closed);
            }

            // Check if enough permits are available
            if curr < n_shifted {
                return Err(TryAcquireError::NoPermits);
            }

            let next = curr - n_shifted;
            match self.permits.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(RankedSemaphorePermit {
                        sem: self,
                        permits: n,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    /// Forget (remove) permits from the semaphore
    ///
    /// Returns the number of permits that were actually removed.
    /// If there are insufficient permits, removes as many as possible.
    pub fn forget_permits(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }

        let mut curr_bits = self.permits.load(Ordering::Acquire);
        loop {
            let curr_permits = curr_bits >> Self::PERMIT_SHIFT;
            let removed = curr_permits.min(n);
            let new_permits = curr_permits - removed;
            let new_bits = (new_permits << Self::PERMIT_SHIFT) | (curr_bits & Self::CLOSED);

            match self.permits.compare_exchange_weak(
                curr_bits,
                new_bits,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return removed,
                Err(actual) => curr_bits = actual,
            }
        }
    }

    // === Owned acquire methods ===

    /// Acquire owned permit (default priority)
    pub fn acquire_owned(self: Arc<Self>) -> AcquireOwned {
        self.acquire_many_owned_with_priority(0, 1)
    }

    /// Acquire owned permit with priority
    pub fn acquire_owned_with_priority(self: Arc<Self>, priority: isize) -> AcquireOwned {
        self.acquire_many_owned_with_priority(priority, 1)
    }

    /// Acquire many owned permits (default priority)
    pub fn acquire_many_owned(self: Arc<Self>, n: u32) -> AcquireOwned {
        self.acquire_many_owned_with_priority(0, n)
    }

    /// Acquire many owned permits with priority
    pub fn acquire_many_owned_with_priority(
        self: Arc<Self>,
        priority: isize,
        n: u32,
    ) -> AcquireOwned {
        AcquireOwned {
            semaphore: self,
            permits_needed: n as usize,
            priority,
            waiter_handle: None,
        }
    }

    /// Attempts to acquire a single permit, returning an owned permit if successful.
    ///
    /// The semaphore must be wrapped in an Arc to call this method.
    /// If the semaphore has been closed, this returns a TryAcquireError::Closed
    /// and a TryAcquireError::NoPermits if there are no permits left.
    /// Otherwise, this returns an OwnedRankedSemaphorePermit representing the acquired permit.
    pub fn try_acquire_owned(
        self: Arc<Self>,
    ) -> Result<OwnedRankedSemaphorePermit, TryAcquireError> {
        let mut curr = self.permits.load(Ordering::Acquire);
        loop {
            // Check if semaphore is closed
            if curr & Self::CLOSED == Self::CLOSED {
                return Err(TryAcquireError::Closed);
            }

            // Check if enough permits are available (need at least 1 << PERMIT_SHIFT)
            if curr < (1 << Self::PERMIT_SHIFT) {
                return Err(TryAcquireError::NoPermits);
            }

            let next = curr - (1 << Self::PERMIT_SHIFT);
            match self.permits.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(OwnedRankedSemaphorePermit {
                        sem: self,
                        permits: 1,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    /// Attempts to acquire n permits, returning an owned permit if successful.
    ///
    /// The semaphore must be wrapped in an Arc to call this method.
    /// If the semaphore has been closed, this returns a TryAcquireError::Closed
    /// and a TryAcquireError::NoPermits if there are not enough permits left.
    /// Otherwise, this returns an OwnedRankedSemaphorePermit representing the acquired permit.
    pub fn try_acquire_many_owned(
        self: Arc<Self>,
        n: u32,
    ) -> Result<OwnedRankedSemaphorePermit, TryAcquireError> {
        if n == 0 {
            return Ok(OwnedRankedSemaphorePermit {
                sem: self,
                permits: 0,
            });
        }

        if n as usize > Self::MAX_PERMITS {
            panic!("try_acquire_many_owned: n exceeds MAX_PERMITS");
        }

        let n_shifted = (n as usize) << Self::PERMIT_SHIFT;
        let mut curr = self.permits.load(Ordering::Acquire);
        loop {
            // Check if semaphore is closed
            if curr & Self::CLOSED == Self::CLOSED {
                return Err(TryAcquireError::Closed);
            }

            // Check if enough permits are available
            if curr < n_shifted {
                return Err(TryAcquireError::NoPermits);
            }

            let next = curr - n_shifted;
            match self.permits.compare_exchange_weak(
                curr,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(OwnedRankedSemaphorePermit {
                        sem: self,
                        permits: n,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }
}

// === Future Implementation ===

impl<'a> Future for Acquire<'a> {
    type Output = Result<RankedSemaphorePermit<'a>, AcquireError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        // Fast path
        if this.waiter_handle.is_none() {
            match this.semaphore.try_acquire_many(this.permits_needed as u32) {
                Ok(permit) => return Poll::Ready(Ok(permit)),
                Err(TryAcquireError::NoPermits) => {
                    // Continue to slow path
                }
                Err(TryAcquireError::Closed) => return Poll::Ready(Err(AcquireError::closed())),
            }
        }

        // Slow path: queue the waiter
        if this.waiter_handle.is_none() {
            let mut waiters = this.semaphore.waiters.lock().unwrap();
            // Re-check permits under lock
            match this.semaphore.try_acquire_many(this.permits_needed as u32) {
                Ok(permit) => return Poll::Ready(Ok(permit)),
                Err(TryAcquireError::NoPermits) => {
                    if this.semaphore.is_closed() {
                        return Poll::Ready(Err(AcquireError::closed()));
                    }
                    this.waiter_handle =
                        Some(waiters.push_waiter(this.permits_needed, this.priority));
                }
                Err(TryAcquireError::Closed) => return Poll::Ready(Err(AcquireError::closed())),
            }
        }

        // Wait for notification
        let handle = this.waiter_handle.as_ref().unwrap();
        handle.state.set_waker(cx.waker().clone());

        if handle.state.is_notified() {
            // The permit was reserved for us by `add_permits`.
            return Poll::Ready(Ok(RankedSemaphorePermit {
                sem: this.semaphore,
                permits: this.permits_needed as u32,
            }));
        }

        if handle.state.is_cancelled() {
            return Poll::Ready(Err(AcquireError::closed()));
        }

        Poll::Pending
    }
}

impl<'a> Drop for Acquire<'a> {
    fn drop(&mut self) {
        // If the future is dropped while in the wait queue, we need to cancel it.
        if let Some(handle) = &self.waiter_handle {
            handle.state.cancel();
        }
    }
}

impl Future for AcquireOwned {
    type Output = Result<OwnedRankedSemaphorePermit, AcquireError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        // Fast path
        if this.waiter_handle.is_none() {
            match this.semaphore.try_acquire_many(this.permits_needed as u32) {
                Ok(permit) => {
                    // This is a bit tricky. `try_acquire_many` returns a borrowed permit.
                    // We need an owned one. We can "forget" the borrowed one and create an owned one.
                    let permits = permit.permits;
                    std::mem::forget(permit);
                    return Poll::Ready(Ok(OwnedRankedSemaphorePermit {
                        sem: Arc::clone(&this.semaphore),
                        permits,
                    }));
                }
                Err(TryAcquireError::NoPermits) => {
                    // Continue to slow path
                }
                Err(TryAcquireError::Closed) => return Poll::Ready(Err(AcquireError::closed())),
            }
        }

        // Slow path: queue the waiter
        if this.waiter_handle.is_none() {
            let mut waiters = this.semaphore.waiters.lock().unwrap();
            // Re-check permits under lock
            match this.semaphore.try_acquire_many(this.permits_needed as u32) {
                Ok(permit) => {
                    let permits = permit.permits;
                    std::mem::forget(permit);
                    return Poll::Ready(Ok(OwnedRankedSemaphorePermit {
                        sem: Arc::clone(&this.semaphore),
                        permits,
                    }));
                }
                Err(TryAcquireError::NoPermits) => {
                    if this.semaphore.is_closed() {
                        return Poll::Ready(Err(AcquireError::closed()));
                    }
                    this.waiter_handle =
                        Some(waiters.push_waiter(this.permits_needed, this.priority));
                }
                Err(TryAcquireError::Closed) => return Poll::Ready(Err(AcquireError::closed())),
            }
        }

        // Wait for notification
        let handle = this.waiter_handle.as_ref().unwrap();
        handle.state.set_waker(cx.waker().clone());

        if handle.state.is_notified() {
            // The permit was reserved for us by `add_permits`.
            return Poll::Ready(Ok(OwnedRankedSemaphorePermit {
                sem: Arc::clone(&this.semaphore),
                permits: this.permits_needed as u32,
            }));
        }

        if handle.state.is_cancelled() {
            return Poll::Ready(Err(AcquireError::closed()));
        }

        Poll::Pending
    }
}

impl Drop for AcquireOwned {
    fn drop(&mut self) {
        // If the future is dropped while in the wait queue, we need to cancel it.
        if let Some(handle) = &self.waiter_handle {
            handle.state.cancel();
        }
    }
}

// === Permit Implementation ===

impl<'a> RankedSemaphorePermit<'a> {
    /// Forget permit (does not release back to semaphore)
    pub fn forget(mut self) {
        self.permits = 0;
    }

    /// Get number of permits
    pub fn num_permits(&self) -> usize {
        self.permits as usize
    }

    /// Merges another permit into this one. Both permits must belong to the same semaphore.
    pub fn merge(&mut self, mut other: Self) {
        if !std::ptr::eq(self.sem, other.sem) {
            panic!("Cannot merge permits from different semaphores");
        }
        self.permits += other.permits;
        // Prevent double drop
        other.permits = 0;
    }

    /// Splits n permits from this permit, returning a new permit.
    pub fn split(&mut self, n: u32) -> Option<Self> {
        if n > self.permits {
            return None;
        }
        self.permits -= n;
        Some(Self {
            sem: self.sem,
            permits: n,
        })
    }
}

impl<'a> Drop for RankedSemaphorePermit<'a> {
    fn drop(&mut self) {
        if self.permits == 0 {
            return;
        }

        // Try fast path first - most common case is no waiters
        let permits_to_add = (self.permits as usize) << RankedSemaphore::PERMIT_SHIFT;
        let waiters = self.sem.waiters.lock().unwrap();

        if waiters.is_empty() {
            // No waiters, just add permits back atomically after releasing lock
            drop(waiters);
            self.sem
                .permits
                .fetch_add(permits_to_add, Ordering::Release);
            return;
        }

        // There are waiters, use add_permits_locked directly with the lock we already have
        self.sem.add_permits_locked(self.permits as usize, waiters);
    }
}

impl OwnedRankedSemaphorePermit {
    /// Forget permit (does not release back to semaphore)
    pub fn forget(mut self) {
        self.permits = 0;
    }

    /// Get number of permits
    pub fn num_permits(&self) -> usize {
        self.permits as usize
    }

    /// Merges another permit into this one. Both permits must belong to the same semaphore.
    pub fn merge(&mut self, mut other: Self) {
        if !Arc::ptr_eq(&self.sem, &other.sem) {
            panic!("merging permits from different semaphore instances");
        }
        self.permits += other.permits;
        // Prevent double drop
        other.permits = 0;
    }

    /// Splits n permits from this permit, returning a new owned permit.
    /// If there are insufficient permits and it's not possible to reduce by n, returns None.
    pub fn split(&mut self, n: usize) -> Option<Self> {
        let n = u32::try_from(n).ok()?;

        if n > self.permits {
            return None;
        }

        self.permits -= n;

        Some(Self {
            sem: self.sem.clone(),
            permits: n,
        })
    }

    /// Returns the Semaphore from which this permit was acquired.
    pub fn semaphore(&self) -> &Arc<RankedSemaphore> {
        &self.sem
    }
}

impl Drop for OwnedRankedSemaphorePermit {
    fn drop(&mut self) {
        if self.permits == 0 {
            return;
        }

        // Try fast path first - most common case is no waiters
        let permits_to_add = (self.permits as usize) << RankedSemaphore::PERMIT_SHIFT;
        let waiters = self.sem.waiters.lock().unwrap();

        if waiters.is_empty() {
            // No waiters, just add permits back atomically after releasing lock
            drop(waiters);
            self.sem
                .permits
                .fetch_add(permits_to_add, Ordering::Release);
            return;
        }

        // There are waiters, use add_permits_locked directly with the lock we already have
        self.sem.add_permits_locked(self.permits as usize, waiters);
    }
}

// === Debug Implementation ===

impl<'a> fmt::Debug for RankedSemaphorePermit<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RankedSemaphorePermit")
            .field("permits", &self.permits)
            .finish()
    }
}

impl fmt::Debug for OwnedRankedSemaphorePermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedRankedSemaphorePermit")
            .field("permits", &self.permits)
            .finish()
    }
}

impl<'a> fmt::Debug for Acquire<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Acquire")
            .field("permits_needed", &self.permits_needed)
            .field("priority", &self.priority)
            .field("queued", &self.waiter_handle.is_some())
            .finish()
    }
}

impl fmt::Debug for AcquireOwned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcquireOwned")
            .field("permits_needed", &self.permits_needed)
            .field("priority", &self.priority)
            .field("queued", &self.waiter_handle.is_some())
            .finish()
    }
}
