use ranked_semaphore::RankedSemaphore;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;

/// Tests the fast path of `try_acquire`.
/// This test ensures that when there is no contention, `try_acquire` succeeds
/// immediately, implicitly testing the `compare_exchange` fast path.
#[tokio::test]
async fn test_try_acquire_fast_path_no_contention() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));
    let permit = sem.try_acquire().unwrap();
    assert_eq!(permit.num_permits(), 1);
    assert!(
        sem.try_acquire().is_err(),
        "Should not be able to acquire more permits"
    );
}

/// Tests the race condition between `add_permits` and `close`.
#[tokio::test]
async fn test_concurrent_add_permits_and_close() {
    let sem = Arc::new(RankedSemaphore::new_fifo(0));
    let sem_clone1 = sem.clone();
    let sem_clone2 = sem.clone();
    let sem_clone3 = sem.clone();

    let acquire_task = tokio::spawn(async move {
        // This task will be suspended as there are no permits.
        sem_clone1.acquire_owned().await
    });

    // Give the acquire_task time to enter the wait queue.
    time::sleep(Duration::from_millis(50)).await;

    // Start adding permits in the background
    let add_permits_task = tokio::spawn(async move {
        sem_clone2.add_permits(1);
    });

    // Close immediately after adding permits
    let close_task = tokio::spawn(async move {
        sem_clone3.close();
    });

    // Wait for both operations
    let _ = tokio::join!(add_permits_task, close_task);

    // The acquire_task should eventually return an error because the semaphore was closed.
    let result = acquire_task.await.unwrap();
    assert!(
        result.is_err(),
        "Acquire should fail when the semaphore is closed"
    );
}

/// Tests the behavior of an `Acquire` future being dropped after entering the wait queue.
#[tokio::test]
async fn test_acquire_future_dropped_from_wait_queue() {
    let sem = Arc::new(RankedSemaphore::new_fifo(0));
    let sem_clone = sem.clone();

    let acquire_task = tokio::spawn(async move { sem_clone.acquire_owned().await });

    // Wait for the acquire_task to enter the wait queue.
    time::sleep(Duration::from_millis(50)).await;

    // Drop the future by aborting the task.
    acquire_task.abort();

    // Give some time for the drop handler to run and clean up
    time::sleep(Duration::from_millis(10)).await;

    // The semaphore should not have any waiters now.
    // We can verify this by adding a permit and trying to acquire it.
    sem.add_permits(1);
    let permit = sem
        .try_acquire()
        .expect("Should be able to acquire a permit");
    assert_eq!(permit.num_permits(), 1);
}

/// Tests that `add_permits` panics on overflow.
#[test]
#[should_panic(expected = "would overflow MAX_PERMITS")]
fn test_add_permits_overflow_panic() {
    let sem = RankedSemaphore::new_fifo(RankedSemaphore::MAX_PERMITS);
    sem.add_permits(1); // This should panic.
}
