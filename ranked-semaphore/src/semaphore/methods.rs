use crate::error::{AcquireError, TryAcquireError};
use crate::semaphore::permits::{OwnedRankedSemaphorePermit, RankedSemaphorePermit};
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl super::RankedSemaphore {
    // === Acquire methods ===

    /// Acquires a single permit with default priority (0).
    ///
    /// This method will wait until a permit becomes available. If the semaphore
    /// is closed while waiting, it returns `AcquireError::Closed`.
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(RankedSemaphorePermit)` - Successfully acquired permit
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(1);
    /// let permit = sem.acquire().await.unwrap();
    /// // Permit is automatically released when dropped
    /// # }
    /// ```
    pub fn acquire(&self) -> impl Future<Output = Result<RankedSemaphorePermit<'_>, AcquireError>> {
        self.acquire_many_with_priority(0, 1)
    }

    /// Acquires a single permit with the specified priority.
    ///
    /// Higher priority values are served first. Tasks with the same priority
    /// are served according to the queue strategy (FIFO or LIFO).
    ///
    /// # Arguments
    ///
    /// * `priority` - The priority level for this acquisition request
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(RankedSemaphorePermit)` - Successfully acquired permit
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(1);
    /// let permit = sem.acquire_with_priority(10).await.unwrap();
    /// # }
    /// ```
    pub fn acquire_with_priority(&self, priority: isize) -> impl Future<Output = Result<RankedSemaphorePermit<'_>, AcquireError>> {
        self.acquire_many_with_priority(priority, 1)
    }

    /// Acquires multiple permits with default priority (0).
    ///
    /// This method will wait until all requested permits become available.
    /// The operation is atomic - either all permits are acquired or none are.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(RankedSemaphorePermit)` - Successfully acquired all permits
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let permits = sem.acquire_many(3).await.unwrap();
    /// assert_eq!(permits.num_permits(), 3);
    /// # }
    /// ```
    pub fn acquire_many(&self, n: u32) -> impl Future<Output = Result<RankedSemaphorePermit<'_>, AcquireError>> {
        self.acquire_many_with_priority(0, n)
    }

    /// Acquires multiple permits with the specified priority.
    ///
    /// This method will wait until all requested permits become available.
    /// The operation is atomic - either all permits are acquired or none are.
    ///
    /// # Arguments
    ///
    /// * `priority` - The priority level for this acquisition request
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(RankedSemaphorePermit)` - Successfully acquired all permits
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let permits = sem.acquire_many_with_priority(10, 3).await.unwrap();
    /// assert_eq!(permits.num_permits(), 3);
    /// # }
    /// ```
    pub fn acquire_many_with_priority(
        &self,
        priority: isize,
        n: u32,
    ) -> impl Future<Output = Result<RankedSemaphorePermit<'_>, AcquireError>> {
        super::futures::Acquire {
            semaphore: self,
            permits_needed: n as usize,
            priority,
            waiter_handle: None,
        }
    }

    // === Non-blocking acquire methods ===

    /// Attempts to acquire a single permit without waiting.
    ///
    /// This method returns immediately, either with a permit or an error.
    /// It will not wait for permits to become available.
    ///
    /// # Returns
    ///
    /// * `Ok(RankedSemaphorePermit)` - Successfully acquired permit
    /// * `Err(TryAcquireError::Closed)` - Semaphore is closed
    /// * `Err(TryAcquireError::NoPermits)` - No permits available
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, TryAcquireError};
    ///
    /// let sem = RankedSemaphore::new_fifo(1);
    /// let _permit1 = sem.try_acquire().unwrap();
    /// 
    /// match sem.try_acquire() {
    ///     Ok(_) => panic!("Should not succeed"),
    ///     Err(TryAcquireError::NoPermits) => println!("No permits available"),
    ///     Err(TryAcquireError::Closed) => println!("Semaphore closed"),
    /// };
    /// ```
    pub fn try_acquire(
        &self,
    ) -> Result<super::permits::RankedSemaphorePermit<'_>, TryAcquireError> {
        // Optimized: single load and combined check
        let curr = self.permits.load(Ordering::Acquire);

        // Quick check: closed and no permits in one go
        if curr & Self::CLOSED != 0 {
            return Err(TryAcquireError::Closed);
        }
        if curr < (1 << Self::PERMIT_SHIFT) {
            return Err(TryAcquireError::NoPermits);
        }

        // Try immediate CAS
        let next = curr - (1 << Self::PERMIT_SHIFT);
        match self
            .permits
            .compare_exchange(curr, next, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(super::permits::RankedSemaphorePermit {
                sem: self,
                permits: 1,
            }),
            Err(_) => {
                // Fall back to original retry loop only if immediate CAS fails
                self.try_acquire_retry_loop()
            }
        }
    }

    /// Retry loop for contended cases
    fn try_acquire_retry_loop(
        &self,
    ) -> Result<super::permits::RankedSemaphorePermit<'_>, TryAcquireError> {
        let mut curr = self.permits.load(Ordering::Acquire);
        loop {
            // Check if semaphore is closed
            if curr & Self::CLOSED == Self::CLOSED {
                return Err(TryAcquireError::Closed);
            }

            // Check if enough permits are available
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
                    return Ok(super::permits::RankedSemaphorePermit {
                        sem: self,
                        permits: 1,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    /// Attempts to acquire multiple permits without waiting.
    ///
    /// This method returns immediately, either with all requested permits or an error.
    /// The operation is atomic - either all permits are acquired or none are.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// * `Ok(RankedSemaphorePermit)` - Successfully acquired all permits
    /// * `Err(TryAcquireError::Closed)` - Semaphore is closed
    /// * `Err(TryAcquireError::NoPermits)` - Insufficient permits available
    ///
    /// # Panics
    ///
    /// Panics if `n` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, TryAcquireError};
    ///
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let _permits = sem.try_acquire_many(3).unwrap();
    /// 
    /// match sem.try_acquire_many(5) {
    ///     Ok(_) => panic!("Should not succeed"),
    ///     Err(TryAcquireError::NoPermits) => println!("Not enough permits"),
    ///     Err(TryAcquireError::Closed) => println!("Semaphore closed"),
    /// };
    /// ```
    pub fn try_acquire_many(
        &self,
        n: u32,
    ) -> Result<super::permits::RankedSemaphorePermit<'_>, TryAcquireError> {
        if n == 0 {
            return Ok(super::permits::RankedSemaphorePermit {
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
                    return Ok(super::permits::RankedSemaphorePermit {
                        sem: self,
                        permits: n,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    // === Owned acquire methods ===

    /// Acquires a single owned permit with default priority (0).
    ///
    /// This method returns an owned permit that holds a reference to the semaphore.
    /// The semaphore must be wrapped in an `Arc` to call this method.
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired permit
    /// - `Err(AcquireError)` - Semaphore was closed
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
    /// let permit = sem.acquire_owned().await.unwrap();
    /// # }
    /// ```
    pub fn acquire_owned(self: Arc<Self>) -> impl Future<Output = Result<OwnedRankedSemaphorePermit, AcquireError>> {
        self.acquire_many_owned_with_priority(0, 1)
    }

    /// Acquires a single owned permit with the specified priority.
    ///
    /// Higher priority values are served first. The semaphore must be wrapped
    /// in an `Arc` to call this method.
    ///
    /// # Arguments
    ///
    /// * `priority` - The priority level for this acquisition request
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired permit
    /// - `Err(AcquireError)` - Semaphore was closed
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
    /// let permit = sem.acquire_owned_with_priority(10).await.unwrap();
    /// # }
    /// ```
    pub fn acquire_owned_with_priority(
        self: Arc<Self>,
        priority: isize,
    ) -> impl Future<Output = Result<OwnedRankedSemaphorePermit, AcquireError>> {
        self.acquire_many_owned_with_priority(priority, 1)
    }

    /// Acquires multiple owned permits with default priority (0).
    ///
    /// This method will wait until all requested permits become available.
    /// The operation is atomic - either all permits are acquired or none are.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired all permits
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = Arc::new(RankedSemaphore::new_fifo(5));
    /// let permits = sem.acquire_many_owned(3).await.unwrap();
    /// assert_eq!(permits.num_permits(), 3);
    /// # }
    /// ```
    pub fn acquire_many_owned(self: Arc<Self>, n: u32) -> impl Future<Output = Result<OwnedRankedSemaphorePermit, AcquireError>> {
        self.acquire_many_owned_with_priority(0, n)
    }

    /// Acquires multiple owned permits with the specified priority.
    ///
    /// This method will wait until all requested permits become available.
    /// The operation is atomic - either all permits are acquired or none are.
    ///
    /// # Arguments
    ///
    /// * `priority` - The priority level for this acquisition request
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// A future that resolves to either:
    /// - `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired all permits
    /// - `Err(AcquireError)` - Semaphore was closed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = Arc::new(RankedSemaphore::new_fifo(5));
    /// let permits = sem.acquire_many_owned_with_priority(10, 3).await.unwrap();
    /// assert_eq!(permits.num_permits(), 3);
    /// # }
    /// ```
    pub fn acquire_many_owned_with_priority(
        self: Arc<Self>,
        priority: isize,
        n: u32,
    ) -> impl Future<Output = Result<OwnedRankedSemaphorePermit, AcquireError>> {
        super::futures::AcquireOwned {
            semaphore: self,
            permits_needed: n as usize,
            priority,
            waiter_handle: None,
        }
    }

    /// Attempts to acquire a single owned permit without waiting.
    ///
    /// This method returns immediately, either with a permit or an error.
    /// The semaphore must be wrapped in an `Arc` to call this method.
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired permit
    /// * `Err(TryAcquireError::Closed)` - Semaphore is closed
    /// * `Err(TryAcquireError::NoPermits)` - No permits available
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, TryAcquireError};
    /// use std::sync::Arc;
    ///
    /// let sem = Arc::new(RankedSemaphore::new_fifo(1));
    /// let _permit1 = sem.clone().try_acquire_owned().unwrap();
    /// 
    /// match sem.try_acquire_owned() {
    ///     Ok(_) => panic!("Should not succeed"),
    ///     Err(TryAcquireError::NoPermits) => println!("No permits available"),
    ///     Err(TryAcquireError::Closed) => println!("Semaphore closed"),
    /// }
    /// ```
    pub fn try_acquire_owned(
        self: Arc<Self>,
    ) -> Result<super::permits::OwnedRankedSemaphorePermit, TryAcquireError> {
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
                    return Ok(super::permits::OwnedRankedSemaphorePermit {
                        sem: self,
                        permits: 1,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }

    /// Attempts to acquire multiple owned permits without waiting.
    ///
    /// This method returns immediately, either with all requested permits or an error.
    /// The operation is atomic - either all permits are acquired or none are.
    /// The semaphore must be wrapped in an `Arc` to call this method.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to acquire
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedRankedSemaphorePermit)` - Successfully acquired all permits
    /// * `Err(TryAcquireError::Closed)` - Semaphore is closed
    /// * `Err(TryAcquireError::NoPermits)` - Insufficient permits available
    ///
    /// # Panics
    ///
    /// Panics if `n` exceeds `MAX_PERMITS` (usize::MAX >> 3).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{RankedSemaphore, TryAcquireError};
    /// use std::sync::Arc;
    ///
    /// let sem = Arc::new(RankedSemaphore::new_fifo(5));
    /// let _permits = sem.clone().try_acquire_many_owned(3).unwrap();
    /// 
    /// match sem.try_acquire_many_owned(5) {
    ///     Ok(_) => panic!("Should not succeed"),
    ///     Err(TryAcquireError::NoPermits) => println!("Not enough permits"),
    ///     Err(TryAcquireError::Closed) => println!("Semaphore closed"),
    /// }
    /// ```
    pub fn try_acquire_many_owned(
        self: Arc<Self>,
        n: u32,
    ) -> Result<super::permits::OwnedRankedSemaphorePermit, TryAcquireError> {
        if n == 0 {
            return Ok(super::permits::OwnedRankedSemaphorePermit {
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
                    return Ok(super::permits::OwnedRankedSemaphorePermit {
                        sem: self,
                        permits: n,
                    })
                }
                Err(actual) => curr = actual,
            }
        }
    }
}
