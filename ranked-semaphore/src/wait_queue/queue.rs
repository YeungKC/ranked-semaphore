use crate::config::{PriorityConfig, QueueStrategy};
use crate::wait_queue::waker::WakeList;
use crate::wait_queue::waiter::{WaiterHandle, WaiterState};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type PriorityQueues = HashMap<isize, VecDeque<Arc<WaiterState>>>;

trait PriorityQueueExt {
    fn get_mut_or_create(&mut self, priority: isize) -> &mut VecDeque<Arc<WaiterState>>;
    fn has_non_empty_queue(&self, priority: isize) -> bool;
    fn drain_all_waiters(&mut self) -> Vec<Arc<WaiterState>>;
}

impl PriorityQueueExt for PriorityQueues {
    fn get_mut_or_create(&mut self, priority: isize) -> &mut VecDeque<Arc<WaiterState>> {
        self.entry(priority).or_insert_with(|| VecDeque::with_capacity(4))
    }

    fn has_non_empty_queue(&self, priority: isize) -> bool {
        self.get(&priority).is_some_and(|q| !q.is_empty())
    }

    fn drain_all_waiters(&mut self) -> Vec<Arc<WaiterState>> {
        let mut all_waiters = Vec::new();
        for (_, queue) in self.drain() {
            all_waiters.extend(queue);
        }
        all_waiters
    }
}

/// Priority wait queue
pub(crate) struct WaitQueue {
    priority_queues: PriorityQueues,
    active_priorities: BinaryHeap<isize>,
    config: PriorityConfig,
    closed: AtomicBool,
    total_waiters: usize,
}

// Safety: No raw pointers, uses Arc
unsafe impl Send for WaitQueue {}
unsafe impl Sync for WaitQueue {}

impl WaitQueue {
    pub(crate) fn new(config: PriorityConfig) -> Self {
        Self {
            priority_queues: HashMap::new(),
            active_priorities: BinaryHeap::new(),
            config,
            closed: AtomicBool::new(false),
            total_waiters: 0,
        }
    }

    pub(crate) fn push_waiter(&mut self, permits_needed: usize, priority: isize) -> WaiterHandle {
        let waiter_handle = WaiterHandle::new(permits_needed, priority);

        // Insert waiter into the appropriate queue
        self.insert_waiter_optimized(waiter_handle.state.clone(), priority);

        waiter_handle
    }

    /// Select waiters to notify
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

        // Fast path: single priority (common case)
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

            // Remove cancelled waiters efficiently
            if waiter.is_cancelled() {
                queue.pop_front();
                self.total_waiters -= 1;
                continue;
            }

            // Quick check: if this waiter needs more permits than available, we're done
            if waiter.permits_needed > available_permits {
                break;
            }

            // Process valid waiter
            let waiter_to_notify = queue.pop_front().unwrap();
            available_permits -= waiter_to_notify.permits_needed;
            *permits_assigned += waiter_to_notify.permits_needed;
            self.total_waiters -= 1;

            // Optimized notification: combine state check and waker extraction using safe wrapper
            if waiter_to_notify.try_notify() {
                // Safety: We have exclusive access to the wait queue via the mutex in semaphore.rs
                // No other thread can be accessing wakers at this point
                unsafe {
                    if let Some(waker) = waiter_to_notify.take_waker_under_lock() {
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

    fn process_multiple_priorities_optimized(
        &mut self,
        mut available_permits: usize,
        wake_list: &mut WakeList,
        permits_assigned: &mut usize,
    ) {
        let mut priorities_to_restore = Vec::with_capacity(4);

        while let Some(&priority) = self.active_priorities.peek() {
            if available_permits == 0 || !wake_list.can_push() {
                break;
            }

            // Check if we have a non-empty queue for this priority
            if !self.priority_queues.has_non_empty_queue(priority) {
                self.active_priorities.pop();
                continue;
            }

            // Get the queue to check the first waiter
            let queue = self.priority_queues.get_mut(&priority).unwrap();
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

                // Optimized notification using safe wrapper
                if waiter_to_notify.try_notify() {
                    // Safety: We have exclusive access to the wait queue via the mutex in semaphore.rs
                    // No other thread can be accessing wakers at this point
                    unsafe {
                        if let Some(waker) = waiter_to_notify.take_waker_under_lock() {
                            if wake_list.can_push() {
                                wake_list.push_unchecked(waker);
                            } else {
                                wake_list.push(waker);
                            }
                        }
                    }
                }
            }

            // Remove any remaining cancelled waiters from the queue
            let before_count = queue.len();
            queue.retain(|waiter| !waiter.is_cancelled());
            let after_count = queue.len();
            let cancelled_count = before_count - after_count;
            if self.total_waiters >= cancelled_count {
                self.total_waiters -= cancelled_count;
            }

            // Keep priority active if queue is not empty
            if !queue.is_empty() {
                priorities_to_restore.push(priority);
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

    pub(crate) fn close(&mut self) {
        self.closed.store(true, Ordering::Release);

        let all_waiters = self.priority_queues.drain_all_waiters();

        self.active_priorities.clear();
        self.total_waiters = 0;

        for waiter in all_waiters {
            waiter.cancel();
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.total_waiters == 0
    }
    
    pub(crate) fn remove_waiter(&mut self, waiter: &Arc<WaiterState>) {
        let priority = waiter.priority;
        
        if let Some(queue) = self.priority_queues.get_mut(&priority) {
            if let Some(pos) = queue.iter().position(|w| Arc::ptr_eq(w, waiter)) {
                queue.remove(pos);
                if self.total_waiters > 0 {
                    self.total_waiters -= 1;
                }
            }
        }
    }

    fn insert_waiter_optimized(&mut self, waiter: Arc<WaiterState>, priority: isize) {
        let strategy = self.config.resolve_strategy(priority);

        // Check if we need to activate this priority
        let was_empty = !self.priority_queues.has_non_empty_queue(priority);

        let queue = self.priority_queues.get_mut_or_create(priority);

        if was_empty {
            self.active_priorities.push(priority);
        }

        match strategy {
            QueueStrategy::Fifo => queue.push_back(waiter),
            QueueStrategy::Lifo => queue.push_front(waiter),
        }

        self.total_waiters += 1;
    }
}

impl fmt::Debug for WaitQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitQueue")
            .field("total_waiters", &self.total_waiters)
            .field("active_priorities", &self.active_priorities.len())
            .field(
                "priority_queues",
                &self.priority_queues.len(),
            )
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish()
    }
}