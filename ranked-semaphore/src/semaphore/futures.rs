use crate::error::AcquireError;
use crate::wait_queue::waiter::WaiterHandle;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// A future representing an ongoing permit acquisition operation.
///
/// This future is returned by the various `acquire*` methods on [`RankedSemaphore`].
/// It will resolve when the requested permits become available or when the
/// semaphore is closed.
///
/// # Examples
///
/// ```rust
/// use ranked_semaphore::RankedSemaphore;
///
/// # #[tokio::main]
/// # async fn main() {
/// let sem = RankedSemaphore::new_fifo(1);
/// let acquire_future = sem.acquire();
/// let permit = acquire_future.await.unwrap();
/// # }
/// ```
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub(crate) struct Acquire<'a> {
    pub(crate) semaphore: &'a super::RankedSemaphore,
    pub(crate) permits_needed: usize,
    pub(crate) priority: isize,
    pub(crate) waiter_handle: Option<WaiterHandle>,
}

/// A future representing an ongoing owned permit acquisition operation.
///
/// This future is returned by the various `acquire*_owned` methods on [`RankedSemaphore`].
/// It will resolve when the requested permits become available or when the
/// semaphore is closed.
///
/// # Examples
///
/// ```rust
/// use ranked_semaphore::RankedSemaphore;
/// use std::sync::Arc;
///
/// # #[tokio::main]
/// # async fn main() {
/// let sem = Arc::new(RankedSemaphore::new_fifo(1));
/// let acquire_future = sem.acquire_owned();
/// let permit = acquire_future.await.unwrap();
/// # }
/// ```
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub(crate) struct AcquireOwned {
    pub(crate) semaphore: Arc<super::RankedSemaphore>,
    pub(crate) permits_needed: usize,
    pub(crate) priority: isize,
    pub(crate) waiter_handle: Option<WaiterHandle>,
}

impl<'a> Future for Acquire<'a> {
    type Output = Result<super::RankedSemaphorePermit<'a>, AcquireError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        // Simple fast path - single attempt
        if this.waiter_handle.is_none() {
            let n_shifted = (this.permits_needed) << super::RankedSemaphore::PERMIT_SHIFT;
            let curr = this.semaphore.permits.load(std::sync::atomic::Ordering::Relaxed);
            
            // Quick check: closed or insufficient permits
            if curr & super::RankedSemaphore::CLOSED != 0 {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            if curr >= n_shifted {
                // Single CAS attempt
                let next = curr - n_shifted;
                if this.semaphore.permits.compare_exchange_weak(
                    curr,
                    next,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Relaxed,
                ).is_ok() {
                    return Poll::Ready(Ok(super::RankedSemaphorePermit {
                        sem: this.semaphore,
                        permits: this.permits_needed as u32,
                    }));
                }
            }
            // Fast path failed, proceed to slow path
        }

        // Minimal slow path: just queue and wait
        if this.waiter_handle.is_none() {
            let mut waiters = this.semaphore.waiters.lock().unwrap();
            
            // Final check for closed state
            if this.semaphore.permits.load(std::sync::atomic::Ordering::Relaxed) & super::RankedSemaphore::CLOSED != 0 {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            // Queue immediately - no retry under lock
            this.waiter_handle = Some(waiters.push_waiter(this.permits_needed, this.priority));
        }

        // Wait for notification (optimized waker handling)
        let handle = this.waiter_handle.as_ref().unwrap();
        
        // Check cancelled first to handle the race condition where a waiter
        // might be both notified and cancelled due to semaphore close
        if handle.state.is_cancelled() {
            return Poll::Ready(Err(AcquireError::closed()));
        }
        
        // Then check notification status
        if handle.state.is_notified() {
            // Double-check that the semaphore is still open before returning success
            // This handles the race condition where add_permits notifies us but
            // then close() is called before we poll
            if this.semaphore.is_closed() {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            // The permit was reserved for us by `add_permits`.
            return Poll::Ready(Ok(super::RankedSemaphorePermit {
                sem: this.semaphore,
                permits: this.permits_needed as u32,
            }));
        }

        // Only clone waker if we're actually going to wait
        handle.state.set_waker(cx.waker().clone());
        Poll::Pending
    }
}

impl<'a> Drop for Acquire<'a> {
    fn drop(&mut self) {
        // If the future is dropped while in the wait queue, we need to cancel it.
        if let Some(handle) = self.waiter_handle.take() {
            handle.state.cancel();
            
            // Only remove from queue if it's still waiting (not notified)
            // This avoids expensive O(n) removal for completed waiters
            if handle.state.is_waiting() {
                let mut waiters = self.semaphore.waiters.lock().unwrap();
                waiters.remove_waiter(&handle.state);
            }
        }
    }
}

impl Future for AcquireOwned {
    type Output = Result<super::OwnedRankedSemaphorePermit, AcquireError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        // Simple fast path - single attempt
        if this.waiter_handle.is_none() {
            let n_shifted = (this.permits_needed) << super::RankedSemaphore::PERMIT_SHIFT;
            let curr = this.semaphore.permits.load(std::sync::atomic::Ordering::Relaxed);
            
            // Quick check: closed or insufficient permits
            if curr & super::RankedSemaphore::CLOSED != 0 {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            if curr >= n_shifted {
                // Single CAS attempt
                let next = curr - n_shifted;
                if this.semaphore.permits.compare_exchange_weak(
                    curr,
                    next,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Relaxed,
                ).is_ok() {
                    return Poll::Ready(Ok(super::OwnedRankedSemaphorePermit {
                        sem: Arc::clone(&this.semaphore),
                        permits: this.permits_needed as u32,
                    }));
                }
            }
            // Fast path failed, proceed to slow path
        }

        // Minimal slow path: just queue and wait
        if this.waiter_handle.is_none() {
            let mut waiters = this.semaphore.waiters.lock().unwrap();
            
            // Final check for closed state
            if this.semaphore.permits.load(std::sync::atomic::Ordering::Relaxed) & super::RankedSemaphore::CLOSED != 0 {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            // Queue immediately - no retry under lock
            this.waiter_handle = Some(waiters.push_waiter(this.permits_needed, this.priority));
        }

        // Wait for notification (optimized waker handling)
        let handle = this.waiter_handle.as_ref().unwrap();
        
        // Check cancelled first to handle the race condition where a waiter
        // might be both notified and cancelled due to semaphore close
        if handle.state.is_cancelled() {
            return Poll::Ready(Err(AcquireError::closed()));
        }
        
        // Then check notification status
        if handle.state.is_notified() {
            // Double-check that the semaphore is still open before returning success
            // This handles the race condition where add_permits notifies us but
            // then close() is called before we poll
            if this.semaphore.is_closed() {
                return Poll::Ready(Err(AcquireError::closed()));
            }
            
            // The permit was reserved for us by `add_permits`.
            return Poll::Ready(Ok(super::OwnedRankedSemaphorePermit {
                sem: Arc::clone(&this.semaphore),
                permits: this.permits_needed as u32,
            }));
        }

        // Only clone waker if we're actually going to wait
        handle.state.set_waker(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for AcquireOwned {
    fn drop(&mut self) {
        // If the future is dropped while in the wait queue, we need to cancel it.
        if let Some(handle) = self.waiter_handle.take() {
            handle.state.cancel();
            
            // Only remove from queue if it's still waiting (not notified)
            // This avoids expensive O(n) removal for completed waiters
            if handle.state.is_waiting() {
                let mut waiters = self.semaphore.waiters.lock().unwrap();
                waiters.remove_waiter(&handle.state);
            }
        }
    }
}

// Debug implementations
use std::fmt;

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