use ranked_semaphore::RankedSemaphore;

#[tokio::test]
async fn test_add_permits() {
    let sem = RankedSemaphore::new_fifo(5);
    assert_eq!(sem.available_permits(), 5);

    sem.add_permits(3);
    assert_eq!(sem.available_permits(), 8);

    // Test adding zero permits
    sem.add_permits(0);
    assert_eq!(sem.available_permits(), 8);

    // Test adding permits to a semaphore with waiters
    let _permit1 = sem.try_acquire().unwrap();
    let _permit2 = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 6);

    sem.add_permits(2);
    assert_eq!(sem.available_permits(), 8);
}

#[tokio::test]
async fn test_forget_permits() {
    let sem = RankedSemaphore::new_fifo(10);
    assert_eq!(sem.available_permits(), 10);

    // Test normal forget
    let forgotten = sem.forget_permits(3);
    assert_eq!(forgotten, 3);
    assert_eq!(sem.available_permits(), 7);

    // Test forget more than available
    let forgotten = sem.forget_permits(10);
    assert_eq!(forgotten, 7); // Only 7 were available
    assert_eq!(sem.available_permits(), 0);

    // Test forget zero permits
    let forgotten = sem.forget_permits(0);
    assert_eq!(forgotten, 0);
    assert_eq!(sem.available_permits(), 0);
}

#[tokio::test]
async fn test_permit_num_permits() {
    let sem = RankedSemaphore::new_fifo(10);

    // Test single permit
    let permit1 = sem.try_acquire().unwrap();
    assert_eq!(permit1.num_permits(), 1);

    // Test multiple permits
    let permit2 = sem.try_acquire_many(5).unwrap();
    assert_eq!(permit2.num_permits(), 5);

    // Test zero permits
    let permit3 = sem.try_acquire_many(0).unwrap();
    assert_eq!(permit3.num_permits(), 0);

    drop(permit1);
    drop(permit2);
    drop(permit3);
    assert_eq!(sem.available_permits(), 10);
}

#[tokio::test]
async fn test_max_permits_edge_cases() {
    // Test creation with maximum permits
    let sem = RankedSemaphore::new_fifo(RankedSemaphore::MAX_PERMITS);
    assert_eq!(sem.available_permits(), RankedSemaphore::MAX_PERMITS);

    // Test that we can acquire and release permits normally
    let permit = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), RankedSemaphore::MAX_PERMITS - 1);

    drop(permit);
    assert_eq!(sem.available_permits(), RankedSemaphore::MAX_PERMITS);
}

#[tokio::test]
async fn test_permit_with_priority() {
    let sem = RankedSemaphore::new_fifo(5);

    // Test that permits acquired with priority work the same as regular permits
    let permit1 = sem.try_acquire().unwrap();
    assert_eq!(permit1.num_permits(), 1);
    assert_eq!(sem.available_permits(), 4);

    let permit2 = sem.try_acquire_many(2).unwrap();
    assert_eq!(permit2.num_permits(), 2);
    assert_eq!(sem.available_permits(), 2);

    drop(permit1);
    drop(permit2);
    assert_eq!(sem.available_permits(), 5);
}

#[tokio::test]
async fn test_add_permits_edge_cases() {
    let sem = RankedSemaphore::new_fifo(0);

    // Add permits when semaphore starts with zero
    sem.add_permits(1);
    assert_eq!(sem.available_permits(), 1);

    // Add a large number of permits
    sem.add_permits(1000);
    assert_eq!(sem.available_permits(), 1001);

    // Acquire some permits then add more
    let _permits = sem.try_acquire_many(500).unwrap();
    assert_eq!(sem.available_permits(), 501);

    sem.add_permits(100);
    assert_eq!(sem.available_permits(), 601);
}
