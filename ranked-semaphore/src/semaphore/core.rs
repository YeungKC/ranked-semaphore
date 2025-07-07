use crate::config::{PriorityConfig, QueueStrategy};
use crate::wait_queue::queue::WaitQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A priority-aware semaphore for controlling access to shared resources.
///
/// RankedSemaphore maintains a count of available permits and allows tasks to
/// acquire permits with different priorities. Higher priority tasks are served
/// before lower priority tasks when permits become available.
///
/// The semaphore supports:
/// - Priority-based scheduling with configurable queue strategies
/// - Multiple permit acquisition
/// - Both borrowed and owned permit types
/// - Graceful shutdown via the `close()` method
///
/// # Examples
///
/// ```rust
/// use ranked_semaphore::RankedSemaphore;
/// use std::sync::Arc;
///
/// # #[tokio::main]
/// # async fn main() {
/// let sem = Arc::new(RankedSemaphore::new_fifo(2));
///
/// // High priority task
/// let high_permit = sem.acquire_with_priority(10).await.unwrap();
///
/// // Low priority task will wait
/// let low_permit = sem.acquire_with_priority(1).await.unwrap();
/// # }
/// ```
#[derive(Debug)]
pub struct RankedSemaphore {
    /// Combined permit count and status flags using bit operations
    /// Bit layout: [permit_count << 1 | closed_flag]
    /// This optimization reduces atomic operations by 50% compared to separate fields
    pub(crate) permits: AtomicUsize,
    /// Wait queue (used only when needed)
    pub(crate) waiters: Mutex<WaitQueue>,
}

impl RankedSemaphore {
    /// Maximum permits (reserve 3 bits for flags, same as tokio)
    pub const MAX_PERMITS: usize = usize::MAX >> 3;

    /// Bit flag constants for state encoding (aligned with tokio)
    pub(crate) const CLOSED: usize = 1;
    pub(crate) const PERMIT_SHIFT: usize = 1;

    /// Creates a new semaphore with FIFO (First In, First Out) queue strategy.
    ///
    /// All waiters regardless of priority will be served in the order they arrive.
    /// This is the most common queue strategy for fair resource allocation.
    ///
    /// # Arguments
    ///
    /// * `permits` - The initial number of permits available
    ///
    /// # Panics
    ///
    /// Panics if `permits` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(3);
    /// assert_eq!(sem.available_permits(), 3);
    /// ```
    pub fn new_fifo(permits: usize) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self::new(permits, QueueStrategy::Fifo)
    }

    /// Creates a new semaphore with LIFO (Last In, First Out) queue strategy.
    ///
    /// All waiters regardless of priority will be served in reverse order of arrival.
    /// This can be useful for scenarios where you want to prioritize recently arrived tasks.
    ///
    /// # Arguments
    ///
    /// * `permits` - The initial number of permits available
    ///
    /// # Panics
    ///
    /// Panics if `permits` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_lifo(3);
    /// assert_eq!(sem.available_permits(), 3);
    /// ```
    pub fn new_lifo(permits: usize) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self::new(permits, QueueStrategy::Lifo)
    }

    /// Creates a new semaphore with the specified default queue strategy.
    ///
    /// All waiters will use the specified queue strategy regardless of their priority.
    /// For more fine-grained control over queue strategies per priority level,
    /// use `new_with_config()`.
    ///
    /// # Arguments
    ///
    /// * `permits` - The initial number of permits available
    /// * `default_strategy` - The queue strategy to use for all waiters
    ///
    /// # Panics
    ///
    /// Panics if `permits` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, QueueStrategy};
    ///
    /// let sem = RankedSemaphore::new(3, QueueStrategy::Fifo);
    /// assert_eq!(sem.available_permits(), 3);
    /// ```
    pub fn new(permits: usize, default_strategy: QueueStrategy) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        let config = PriorityConfig::new().default_strategy(default_strategy);
        Self::new_with_config(permits, config)
    }

    /// Creates a new semaphore with custom priority-based configuration.
    ///
    /// This allows fine-grained control over queue strategies for different
    /// priority levels. Different priorities can use different queue strategies
    /// (FIFO or LIFO) based on the configuration rules.
    ///
    /// # Arguments
    ///
    /// * `permits` - The initial number of permits available
    /// * `config` - The priority configuration specifying queue strategies
    ///
    /// # Panics
    ///
    /// Panics if `permits` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, PriorityConfig, QueueStrategy};
    ///
    /// let config = PriorityConfig::new()
    ///     .default_strategy(QueueStrategy::Fifo)
    ///     .exact(10, QueueStrategy::Lifo); // Priority 10 uses LIFO
    ///
    /// let sem = RankedSemaphore::new_with_config(3, config);
    /// assert_eq!(sem.available_permits(), 3);
    /// ```
    pub fn new_with_config(permits: usize, config: PriorityConfig) -> Self {
        if permits > Self::MAX_PERMITS {
            panic!("permits exceed MAX_PERMITS");
        }
        Self {
            permits: AtomicUsize::new(permits << Self::PERMIT_SHIFT),
            waiters: Mutex::new(WaitQueue::new(config)),
        }
    }

    /// Returns the current number of available permits.
    ///
    /// This value represents permits that can be acquired immediately without waiting.
    /// Note that this value can change rapidly in concurrent environments.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(3);
    /// assert_eq!(sem.available_permits(), 3);
    ///
    /// let _permit = sem.try_acquire().unwrap();
    /// assert_eq!(sem.available_permits(), 2);
    /// ```
    pub fn available_permits(&self) -> usize {
        self.permits.load(Ordering::Acquire) >> Self::PERMIT_SHIFT
    }

    /// Returns `true` if the semaphore has been closed.
    ///
    /// A closed semaphore will not issue new permits and all pending
    /// acquire operations will return `AcquireError::Closed`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(3);
    /// assert!(!sem.is_closed());
    ///
    /// sem.close();
    /// assert!(sem.is_closed());
    /// ```
    pub fn is_closed(&self) -> bool {
        self.permits.load(Ordering::Acquire) & Self::CLOSED == Self::CLOSED
    }

    /// Adds permits to the semaphore and notifies waiting tasks.
    ///
    /// If there are tasks waiting for permits, they will be notified in priority order.
    /// Any excess permits (beyond what waiting tasks need) are added to the
    /// semaphore's available permit count.
    ///
    /// # Arguments
    ///
    /// * `added` - The number of permits to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(1);
    /// let _permit = sem.try_acquire().unwrap();
    /// assert_eq!(sem.available_permits(), 0);
    ///
    /// sem.add_permits(2);
    /// assert_eq!(sem.available_permits(), 2);
    /// ```
    pub fn add_permits(&self, added: usize) {
        if added == 0 {
            return;
        }

        // Assign permits to the wait queue, following tokio's approach
        self.add_permits_locked(added, self.waiters.lock().unwrap());
    }

    /// Add permits to the semaphore and wake eligible waiters.
    ///
    /// If `rem` exceeds the number of permits needed by waiting tasks, the
    /// remainder are returned to the semaphore's available permit count.
    ///
    /// This implementation processes waiters in batches to minimize thundering herd
    /// effects and reduce lock contention.
    pub(crate) fn add_permits_locked(
        &self,
        mut rem: usize,
        waiters: std::sync::MutexGuard<'_, crate::wait_queue::queue::WaitQueue>,
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

            // **Critical check**: if the semaphore was closed while we didn't have the lock,
            // we must not wake up the waiters. They will be woken by the `close()` call.
            if self.is_closed() {
                // Return the permits that were assigned to waiters back to the semaphore
                // since we won't wake them due to the semaphore being closed.
                self.permits
                    .fetch_add(permits_assigned << Self::PERMIT_SHIFT, Ordering::Release);
                return;
            }

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

    /// Closes the semaphore and cancels all pending waiters.
    ///
    /// After calling this method:
    /// - No new permits will be issued
    /// - All pending `acquire` operations will return `AcquireError::Closed`
    /// - All `try_acquire` operations will return `TryAcquireError::Closed`
    ///
    /// This operation is irreversible - once closed, a semaphore cannot be reopened.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(3);
    /// sem.close();
    /// 
    /// assert!(sem.is_closed());
    /// assert!(sem.try_acquire().is_err());
    /// ```
    pub fn close(&self) {
        self.permits.fetch_or(Self::CLOSED, Ordering::Release);

        let mut waiters = self.waiters.lock().unwrap();
        waiters.close();
    }

    /// Permanently removes permits from the semaphore.
    ///
    /// This operation reduces the number of available permits without releasing
    /// them back to the semaphore. It's useful for scenarios where the resource
    /// pool needs to be dynamically reduced.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to remove
    ///
    /// # Returns
    ///
    /// The number of permits that were actually removed. This may be less than
    /// `n` if there were insufficient permits available.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// let sem = RankedSemaphore::new_fifo(5);
    /// assert_eq!(sem.available_permits(), 5);
    ///
    /// let removed = sem.forget_permits(3);
    /// assert_eq!(removed, 3);
    /// assert_eq!(sem.available_permits(), 2);
    ///
    /// // Trying to remove more permits than available
    /// let removed = sem.forget_permits(10);
    /// assert_eq!(removed, 2); // Only 2 were available
    /// assert_eq!(sem.available_permits(), 0);
    /// ```
    ///
    /// # Warning
    ///
    /// Use this method with caution. Removing too many permits can lead to
    /// resource starvation if tasks are waiting for permits that will never
    /// become available.
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
}
