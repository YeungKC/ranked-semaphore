use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A permit that grants access to a resource protected by a semaphore.
///
/// This permit holds a reference to the semaphore and represents one or more
/// permits that have been acquired. When the permit is dropped, the permits
/// are automatically returned to the semaphore.
///
/// # Examples
///
/// ```rust
/// use ranked_semaphore::RankedSemaphore;
///
/// # #[tokio::main]
/// # async fn main() {
/// let sem = RankedSemaphore::new_fifo(3);
/// let permit = sem.acquire().await.unwrap();
/// assert_eq!(permit.num_permits(), 1);
/// // Permit is automatically returned when dropped
/// # }
/// ```
pub struct RankedSemaphorePermit<'a> {
    pub(crate) sem: &'a super::RankedSemaphore,
    pub(crate) permits: u32,
}

/// An owned permit that grants access to a resource protected by a semaphore.
///
/// This permit owns a reference to the semaphore (via `Arc`) and represents
/// one or more permits that have been acquired. When the permit is dropped,
/// the permits are automatically returned to the semaphore.
///
/// # Examples
///
/// ```rust
/// use ranked_semaphore::RankedSemaphore;
/// use std::sync::Arc;
///
/// # #[tokio::main]
/// # async fn main() {
/// let sem = Arc::new(RankedSemaphore::new_fifo(3));
/// let permit = sem.acquire_owned().await.unwrap();
/// assert_eq!(permit.num_permits(), 1);
/// // Permit is automatically returned when dropped
/// # }
/// ```
pub struct OwnedRankedSemaphorePermit {
    pub(crate) sem: Arc<super::RankedSemaphore>,
    pub(crate) permits: u32,
}

impl<'a> RankedSemaphorePermit<'a> {
    /// Forgets this permit without releasing it back to the semaphore.
    ///
    /// This effectively removes the permits from circulation permanently.
    /// Use this method when you want to prevent the permits from being
    /// returned to the semaphore when the permit is dropped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(3);
    /// let permit = sem.acquire().await.unwrap();
    /// 
    /// assert_eq!(sem.available_permits(), 2);
    /// permit.forget(); // Permit is not returned to semaphore
    /// assert_eq!(sem.available_permits(), 2);
    /// # }
    /// ```
    pub fn forget(mut self) {
        self.permits = 0;
    }

    /// Returns the number of permits held by this permit object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let permit = sem.acquire_many(3).await.unwrap();
    /// assert_eq!(permit.num_permits(), 3);
    /// # }
    /// ```
    pub fn num_permits(&self) -> usize {
        self.permits as usize
    }

    /// Merges another permit into this one, combining their permit counts.
    ///
    /// Both permits must belong to the same semaphore instance. After merging,
    /// the other permit becomes invalid (holds 0 permits) and this permit
    /// holds the combined count.
    ///
    /// # Arguments
    ///
    /// * `other` - Another permit from the same semaphore to merge
    ///
    /// # Panics
    ///
    /// Panics if the permits belong to different semaphores.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let mut permit1 = sem.acquire_many(2).await.unwrap();
    /// let permit2 = sem.acquire_many(1).await.unwrap();
    /// 
    /// permit1.merge(permit2);
    /// assert_eq!(permit1.num_permits(), 3);
    /// # }
    /// ```
    pub fn merge(&mut self, mut other: Self) {
        if !std::ptr::eq(self.sem, other.sem) {
            panic!("Cannot merge permits from different semaphores");
        }
        self.permits += other.permits;
        // Prevent double drop
        other.permits = 0;
    }

    /// Splits off a specified number of permits into a new permit.
    ///
    /// This reduces the current permit's count by `n` and returns a new
    /// permit holding `n` permits. If there are insufficient permits,
    /// returns `None`.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to split off
    ///
    /// # Returns
    ///
    /// * `Some(RankedSemaphorePermit)` - New permit holding `n` permits
    /// * `None` - If this permit holds fewer than `n` permits
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = RankedSemaphore::new_fifo(5);
    /// let mut permit = sem.acquire_many(3).await.unwrap();
    /// 
    /// let split_permit = permit.split(2).unwrap();
    /// assert_eq!(permit.num_permits(), 1);
    /// assert_eq!(split_permit.num_permits(), 2);
    /// # }
    /// ```
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
        let permits_to_add = (self.permits as usize) << super::RankedSemaphore::PERMIT_SHIFT;
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
    /// Forgets this permit without releasing it back to the semaphore.
    ///
    /// This effectively removes the permits from circulation permanently.
    /// Use this method when you want to prevent the permits from being
    /// returned to the semaphore when the permit is dropped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = Arc::new(RankedSemaphore::new_fifo(3));
    /// let permit = sem.clone().acquire_owned().await.unwrap();
    /// 
    /// assert_eq!(sem.available_permits(), 2);
    /// permit.forget(); // Permit is not returned to semaphore
    /// assert_eq!(sem.available_permits(), 2);
    /// # }
    /// ```
    pub fn forget(mut self) {
        self.permits = 0;
    }

    /// Returns the number of permits held by this permit object.
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
    /// let permit = sem.acquire_many_owned(3).await.unwrap();
    /// assert_eq!(permit.num_permits(), 3);
    /// # }
    /// ```
    pub fn num_permits(&self) -> usize {
        self.permits as usize
    }

    /// Merges another permit into this one, combining their permit counts.
    ///
    /// Both permits must belong to the same semaphore instance. After merging,
    /// the other permit becomes invalid (holds 0 permits) and this permit
    /// holds the combined count.
    ///
    /// # Arguments
    ///
    /// * `other` - Another permit from the same semaphore to merge
    ///
    /// # Panics
    ///
    /// Panics if the permits belong to different semaphores.
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
    /// let mut permit1 = sem.clone().acquire_many_owned(2).await.unwrap();
    /// let permit2 = sem.acquire_many_owned(1).await.unwrap();
    /// 
    /// permit1.merge(permit2);
    /// assert_eq!(permit1.num_permits(), 3);
    /// # }
    /// ```
    pub fn merge(&mut self, mut other: Self) {
        if !Arc::ptr_eq(&self.sem, &other.sem) {
            panic!("Cannot merge permits from different semaphores");
        }
        self.permits += other.permits;
        // Prevent double drop
        other.permits = 0;
    }

    /// Splits off a specified number of permits into a new owned permit.
    ///
    /// This reduces the current permit's count by `n` and returns a new
    /// owned permit holding `n` permits. If there are insufficient permits,
    /// returns `None`.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of permits to split off
    ///
    /// # Returns
    ///
    /// * `Some(OwnedRankedSemaphorePermit)` - New permit holding `n` permits
    /// * `None` - If this permit holds fewer than `n` permits or `n` doesn't fit in u32
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
    /// let mut permit = sem.acquire_many_owned(3).await.unwrap();
    /// 
    /// let split_permit = permit.split(2).unwrap();
    /// assert_eq!(permit.num_permits(), 1);
    /// assert_eq!(split_permit.num_permits(), 2);
    /// # }
    /// ```
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

    /// Returns a reference to the semaphore from which this permit was acquired.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::RankedSemaphore;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let sem = Arc::new(RankedSemaphore::new_fifo(3));
    /// let permit = sem.acquire_owned().await.unwrap();
    /// 
    /// let sem_ref = permit.semaphore();
    /// assert_eq!(sem_ref.available_permits(), 2);
    /// # }
    /// ```
    pub fn semaphore(&self) -> &Arc<super::RankedSemaphore> {
        &self.sem
    }
}

impl Drop for OwnedRankedSemaphorePermit {
    fn drop(&mut self) {
        if self.permits == 0 {
            return;
        }

        // Try fast path first - most common case is no waiters
        let permits_to_add = (self.permits as usize) << super::RankedSemaphore::PERMIT_SHIFT;
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

// Debug implementations
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