//! High-performance priority wait queue implementation
//!
//! This module implements a high-performance wait queue with the following features:
//! - Dynamic sparse mapping: Memory allocation only for actively used priorities
//! - Stack-allocated wake list: Zero-allocation batch waker processing  
//! - Batch processing: Reduces lock contention and system call overhead
//! - Cache-friendly: Optimized memory layout

use crate::config::PriorityConfig;
use std::cell::UnsafeCell;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::Waker;

/// Maximum number of wakers that can be batched in a single wake operation
/// This matches tokio's WakeList capacity for optimal performance
const NUM_WAKERS: usize = 32;

/// Stack-allocated waker batch list for zero-allocation wake operations
///
/// This is modeled after tokio's WakeList and provides significant performance
/// benefits by avoiding heap allocations during the critical wake path.
pub(crate) struct WakeList {
    wakers: [Option<Waker>; NUM_WAKERS],
    count: usize,
}

impl WakeList {
    /// Create a new empty wake list
    pub(crate) fn new() -> Self {
        Self {
            wakers: [const { None }; NUM_WAKERS],
            count: 0,
        }
    }

    /// Check if the wake list can accept more wakers
    pub(crate) fn can_push(&self) -> bool {
        self.count < NUM_WAKERS
    }

    /// Check if the wake list is empty
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Check if the wake list was at full capacity
    pub(crate) fn was_full(&self) -> bool {
        self.count == NUM_WAKERS
    }

    /// Add a waker to the list
    ///
    /// # Panics
    /// Panics if the list is full. Check with `can_push()` first.
    pub(crate) fn push(&mut self, waker: Waker) {
        debug_assert!(self.can_push(), "WakeList is full");
        self.wakers[self.count] = Some(waker);
        self.count += 1;
    }

    /// Add a waker to the list without bounds checking (unsafe version)
    ///
    /// # Safety
    /// Caller must ensure that `self.count < NUM_WAKERS` before calling this method.
    /// This eliminates the bounds check for maximum performance in hot paths.
    pub(crate) unsafe fn push_unchecked(&mut self, waker: Waker) {
        debug_assert!(self.can_push(), "WakeList is full");
        *self.wakers.get_unchecked_mut(self.count) = Some(waker);
        self.count += 1;
    }

    /// Wake all wakers in the list and clear it
    ///
    /// This method is optimized to avoid panics during waking by using
    /// a structured approach that ensures all wakers are properly cleaned up.
    pub(crate) fn wake_all(&mut self) {
        for i in 0..self.count {
            if let Some(waker) = self.wakers[i].take() {
                waker.wake();
            }
        }
        self.count = 0;
    }

    /// Wake all wakers in the list and clear it (unsafe version)
    ///
    /// # Safety
    /// This method uses unsafe indexing for maximum performance.
    /// The safety is guaranteed by the fact that we only index up to `self.count`,
    /// which is always valid.
    #[allow(dead_code)] // Kept for potential future optimizations
    pub(crate) unsafe fn wake_all_unchecked(&mut self) {
        for i in 0..self.count {
            if let Some(waker) = self.wakers.get_unchecked_mut(i).take() {
                waker.wake();
            }
        }
        self.count = 0;
    }
}

/// High-performance wait queue optimized for contention scenarios
///
/// Uses a HashMap for O(1) queue access and a BinaryHeap for O(log n) priority management.
pub(crate) struct WaitQueue {
    /// Per-priority queues, using a HashMap for fast access.
    priority_queues: HashMap<isize, VecDeque<Arc<WaiterState>>>,
    /// Max-heap to keep track of active priorities, ensuring O(log n) access to the highest priority.
    active_priorities: BinaryHeap<isize>,
    /// Priority configuration
    config: PriorityConfig,
    /// Whether the queue is closed (thread-safe)
    closed: AtomicBool,
    /// Total number of waiters across all priorities
    total_waiters: usize,
}

// Safety: WaitQueue contains no raw pointers and uses Arc for shared ownership
unsafe impl Send for WaitQueue {}
unsafe impl Sync for WaitQueue {}

/// Waiter state (optimized for performance with unsafe access to waker)
///
/// The waker field uses UnsafeCell for maximum performance. All access to the waker
/// must be performed under the protection of the WaitQueue's mutex to ensure safety.
///
/// Memory layout is optimized for cache performance:
/// - Fields are ordered by access frequency (most frequently accessed first)
/// - State field comes first as it's checked most often
pub(crate) struct WaiterState {
    /// Atomic state flags (hot path, accessed most frequently)
    state: AtomicUsize,
    /// Number of permits needed (hot path, accessed frequently)
    pub(crate) permits_needed: usize,
    /// Priority (cold path, accessed less frequently)
    pub(crate) priority: isize,
    /// Task waker (uses UnsafeCell for lock-free access under external mutex protection)
    waker: UnsafeCell<Option<Waker>>,
}

// Safety: WaiterState can be safely sent between threads because:
// 1. All fields except waker are Send
// 2. The waker field is protected by external synchronization (WaitQueue mutex)
// 3. Waker itself is Send + Sync
unsafe impl Send for WaiterState {}

// Safety: WaiterState can be safely accessed from multiple threads because:
// 1. All atomic operations are used for the state field
// 2. Other primitive fields (permits_needed, priority) are immutable after creation
// 3. The waker field uses UnsafeCell but is protected by external mutex
// 4. All unsafe access to waker is properly synchronized
unsafe impl Sync for WaiterState {}

/// Waiter handle for Future to track wait state
pub(crate) struct WaiterHandle {
    /// Strong reference to state
    pub(crate) state: Arc<WaiterState>,
    /// Weak reference for queue cleanup
    _weak_ref: Weak<WaiterState>,
}

// Waiter state constants
const WAITING: usize = 0;
const NOTIFIED: usize = 1;
const CANCELLED: usize = 2;

impl WaitQueue {
    /// Create a new wait queue optimized for high performance
    pub(crate) fn new(config: PriorityConfig) -> Self {
        Self {
            priority_queues: HashMap::new(),
            active_priorities: BinaryHeap::new(),
            config,
            closed: AtomicBool::new(false),
            total_waiters: 0,
        }
    }

    /// Add waiter to queue (O(1) average time for HashMap insertion)
    pub(crate) fn push_waiter(&mut self, permits_needed: usize, priority: isize) -> WaiterHandle {
        let waiter_state = Arc::new(WaiterState {
            state: AtomicUsize::new(WAITING),
            permits_needed,
            priority,
            waker: UnsafeCell::new(None),
        });

        let weak_ref = Arc::downgrade(&waiter_state);

        // Insert waiter into the appropriate queue
        self.insert_waiter_optimized(waiter_state.clone(), priority);

        WaiterHandle {
            state: waiter_state,
            _weak_ref: weak_ref,
        }
    }

    /// Selects a batch of waiters that can be satisfied by the available permits.
    /// This method REMOVES the waiters from the queue and returns a WakeList and permit count for optimal performance.
    ///
    /// Returns (wake_list, permits_assigned) where permits_assigned is the total number of permits
    /// assigned to the woken waiters.
    ///
    /// Optimized for single priority scenarios (most common case).
    pub(crate) fn select_waiters_to_notify(
        &mut self,
        available_permits: usize,
    ) -> (WakeList, usize) {
        let mut wake_list = WakeList::new();
        let mut permits_assigned = 0;

        // Early exit for common cases
        if available_permits == 0 || self.total_waiters == 0 {
            return (wake_list, permits_assigned);
        }

        // Fast path: single priority optimization (most common case)
        if self.active_priorities.len() == 1 {
            if let Some(&priority) = self.active_priorities.peek() {
                // Remove the priority first to avoid borrow conflicts
                self.active_priorities.pop();

                if let Some(mut queue) = self.priority_queues.remove(&priority) {
                    self.process_single_priority_fast(
                        &mut queue,
                        priority,
                        available_permits,
                        &mut wake_list,
                        &mut permits_assigned,
                    );

                    // Put the queue back if not empty
                    if !queue.is_empty() {
                        self.priority_queues.insert(priority, queue);
                        self.active_priorities.push(priority);
                    }
                }
            }
            return (wake_list, permits_assigned);
        }

        // General case: multiple priorities
        self.process_multiple_priorities_optimized(
            available_permits,
            &mut wake_list,
            &mut permits_assigned,
        );
        (wake_list, permits_assigned)
    }

    /// Fast path for single priority processing (eliminates heap operations)
    fn process_single_priority_fast(
        &mut self,
        queue: &mut VecDeque<Arc<WaiterState>>,
        _priority: isize,
        mut available_permits: usize,
        wake_list: &mut WakeList,
        permits_assigned: &mut usize,
    ) {
        // Process waiters from front of queue
        while let Some(waiter) = queue.front() {
            if !wake_list.can_push() || available_permits == 0 {
                break;
            }

            // Quick check: if this waiter needs more permits than available, we're done
            if waiter.permits_needed > available_permits {
                break;
            }

            // Remove cancelled waiters efficiently
            if waiter.is_cancelled() {
                queue.pop_front();
                self.total_waiters -= 1;
                continue;
            }

            // Process valid waiter
            let waiter_to_notify = queue.pop_front().unwrap();
            available_permits -= waiter_to_notify.permits_needed;
            *permits_assigned += waiter_to_notify.permits_needed;
            self.total_waiters -= 1;

            // Optimized notification: combine state check and waker extraction using unsafe access
            if waiter_to_notify
                .state
                .compare_exchange(WAITING, NOTIFIED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Safety: We have exclusive access to the wait queue via the mutex in semaphore.rs
                // No other thread can be accessing wakers at this point
                unsafe {
                    if let Some(waker) = waiter_to_notify.take_waker_unchecked() {
                        if wake_list.can_push() {
                            wake_list.push_unchecked(waker);
                        } else {
                            wake_list.push(waker);
                        }
                    }
                }
            }
        }
    }

    /// Optimized multiple priority processing
    fn process_multiple_priorities_optimized(
        &mut self,
        mut available_permits: usize,
        wake_list: &mut WakeList,
        permits_assigned: &mut usize,
    ) {
        let mut priorities_to_restore = Vec::with_capacity(4); // Optimize for small number of priorities

        while let Some(&priority) = self.active_priorities.peek() {
            if available_permits == 0 || !wake_list.can_push() {
                break;
            }

            let queue = self.priority_queues.get_mut(&priority).unwrap();

            // Remove empty or invalid queues immediately
            if queue.is_empty() {
                self.active_priorities.pop();
                self.priority_queues.remove(&priority);
                continue;
            }

            // Check if the first waiter can be satisfied
            let first_waiter_needs = queue.front().unwrap().permits_needed;
            if first_waiter_needs > available_permits {
                // Can't satisfy highest priority, we're done
                break;
            }

            self.active_priorities.pop();

            // Process this priority's queue
            let mut processed_any = false;
            while let Some(waiter) = queue.front() {
                if !wake_list.can_push() || available_permits == 0 {
                    break;
                }

                if waiter.permits_needed > available_permits {
                    break;
                }

                if waiter.is_cancelled() {
                    queue.pop_front();
                    self.total_waiters -= 1;
                    continue;
                }

                let waiter_to_notify = queue.pop_front().unwrap();
                available_permits -= waiter_to_notify.permits_needed;
                *permits_assigned += waiter_to_notify.permits_needed;
                self.total_waiters -= 1;
                processed_any = true;

                // Optimized notification using unsafe access
                if waiter_to_notify
                    .state
                    .compare_exchange(WAITING, NOTIFIED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    // Safety: We have exclusive access to the wait queue via the mutex in semaphore.rs
                    // No other thread can be accessing wakers at this point
                    unsafe {
                        if let Some(waker) = waiter_to_notify.take_waker_unchecked() {
                            if wake_list.can_push() {
                                wake_list.push_unchecked(waker);
                            } else {
                                wake_list.push(waker);
                            }
                        }
                    }
                }
            }

            // Keep priority active if queue is not empty
            if !queue.is_empty() {
                priorities_to_restore.push(priority);
            } else {
                self.priority_queues.remove(&priority);
            }

            // If we processed at least one waiter but wake_list is full, we need to stop
            if processed_any && !wake_list.can_push() {
                break;
            }
        }

        // Restore active priorities efficiently
        for priority in priorities_to_restore {
            self.active_priorities.push(priority);
        }
    }

    /// Close queue and notify all waiters that they are cancelled.
    pub(crate) fn close(&mut self) {
        self.closed.store(true, Ordering::Release);

        let all_waiters: Vec<_> = self
            .priority_queues
            .values_mut()
            .flat_map(std::mem::take)
            .collect();

        self.priority_queues.clear();
        self.active_priorities.clear();
        self.total_waiters = 0;

        for waiter in all_waiters {
            waiter.cancel();
        }
    }

    /// Check if queue is empty
    pub(crate) fn is_empty(&self) -> bool {
        self.total_waiters == 0
    }

    /// O(1) insertion into per-priority queues with O(log n) for heap management
    fn insert_waiter_optimized(&mut self, waiter: Arc<WaiterState>, priority: isize) {
        use crate::config::QueueStrategy;

        let strategy = self.config.resolve_strategy(priority);

        let queue = self
            .priority_queues
            .entry(priority)
            .or_insert_with(|| VecDeque::with_capacity(8));

        if queue.is_empty() {
            self.active_priorities.push(priority);
        }

        match strategy {
            QueueStrategy::Fifo => queue.push_back(waiter),
            QueueStrategy::Lifo => queue.push_front(waiter),
        }

        self.total_waiters += 1;
    }
}

impl WaiterState {
    /// Cancel waiter (e.g., semaphore closed).
    pub(crate) fn cancel(&self) {
        // Only cancel if it's currently waiting.
        if self
            .state
            .compare_exchange(WAITING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // Safety: This is called during semaphore close, where we have exclusive access
            // to the wait queue and no other threads can be accessing wakers concurrently.
            unsafe {
                if let Some(waker) = (*self.waker.get()).as_ref() {
                    waker.wake_by_ref();
                }
            }
        }
    }

    /// Checks if the waiter has been notified.
    pub(crate) fn is_notified(&self) -> bool {
        self.state.load(Ordering::Relaxed) == NOTIFIED
    }

    /// Checks if the waiter has been cancelled.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Relaxed) == CANCELLED
    }

    /// Sets the task waker safely
    ///
    /// # Safety
    /// This method must only be called when the caller has exclusive access to the
    /// wait queue mutex. This ensures no other thread can be accessing the waker
    /// field concurrently.
    #[allow(dead_code)] // Kept for potential future use
    pub(crate) unsafe fn set_waker_unchecked(&self, waker: Waker) {
        *self.waker.get() = Some(waker);
    }

    /// Takes the waker from the waiter state without synchronization
    ///
    /// # Safety
    /// This method must only be called when the caller has exclusive access to the
    /// wait queue mutex. This ensures no other thread can be accessing the waker
    /// field concurrently.
    pub(crate) unsafe fn take_waker_unchecked(&self) -> Option<Waker> {
        (*self.waker.get()).take()
    }

    /// Sets the task waker (safe version for external use)
    pub(crate) fn set_waker(&self, waker: Waker) {
        // Safety: We use unsafe access here but this is safe because:
        // 1. This method is only called from the Future's poll method
        // 2. At that point, only the current task has access to this waiter
        // 3. The waiter hasn't been added to any shared queue yet
        unsafe {
            *self.waker.get() = Some(waker);
        }
    }
}

impl WaiterHandle {
    // No additional methods needed
}

impl fmt::Debug for WaitQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitQueue")
            .field("total_waiters", &self.total_waiters)
            .field("active_priorities", &self.active_priorities.len())
            .field("priority_queues", &self.priority_queues.len())
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish()
    }
}

impl fmt::Debug for WaiterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Safety: This is only used for debugging and we're just checking if the option is Some
        let has_waker = unsafe { (*self.waker.get()).is_some() };
        f.debug_struct("WaiterState")
            .field("permits_needed", &self.permits_needed)
            .field("priority", &self.priority)
            .field("state", &self.state.load(Ordering::Relaxed))
            .field("has_waker", &has_waker)
            .finish()
    }
}
