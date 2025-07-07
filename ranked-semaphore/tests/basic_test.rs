use ranked_semaphore::{
    AcquireError, PriorityConfig, QueueStrategy, RankedSemaphore, TryAcquireError,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn test_semaphore_creation_fifo() {
    let sem = RankedSemaphore::new_fifo(5);
    assert_eq!(sem.available_permits(), 5);
    assert!(!sem.is_closed());
}

#[tokio::test]
async fn test_semaphore_creation_lifo() {
    let sem = RankedSemaphore::new_lifo(3);
    assert_eq!(sem.available_permits(), 3);
    assert!(!sem.is_closed());
}

#[tokio::test]
async fn test_semaphore_creation_with_config() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .exact(10, QueueStrategy::Lifo);

    let sem = RankedSemaphore::new_with_config(7, config);
    assert_eq!(sem.available_permits(), 7);
    assert!(!sem.is_closed());
}

#[tokio::test]
async fn test_semaphore_creation_zero_permits() {
    let sem = RankedSemaphore::new_fifo(0);
    assert_eq!(sem.available_permits(), 0);
    assert!(!sem.is_closed());

    // try_acquire should fail immediately
    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::NoPermits)));
}

#[tokio::test]
async fn test_semaphore_creation_max_permits() {
    let sem = RankedSemaphore::new_fifo(RankedSemaphore::MAX_PERMITS);
    assert_eq!(sem.available_permits(), RankedSemaphore::MAX_PERMITS);
    assert!(!sem.is_closed());
}

#[tokio::test]
async fn test_try_acquire_success() {
    let sem = RankedSemaphore::new_fifo(3);

    let permit1 = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 2);

    let permit2 = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 1);

    let permit3 = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 0);

    // Clean up
    drop(permit1);
    drop(permit2);
    drop(permit3);
}

#[tokio::test]
async fn test_try_acquire_no_permits() {
    let sem = RankedSemaphore::new_fifo(1);

    let _permit = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 0);

    // Should fail when no permits available
    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::NoPermits)));
}

#[tokio::test]
async fn test_try_acquire_many_success() {
    let sem = RankedSemaphore::new_fifo(10);

    let permit = sem.try_acquire_many(5).unwrap();
    assert_eq!(sem.available_permits(), 5);
    assert_eq!(permit.num_permits(), 5);

    drop(permit);
    assert_eq!(sem.available_permits(), 10);
}

#[tokio::test]
async fn test_try_acquire_many_insufficient_permits() {
    let sem = RankedSemaphore::new_fifo(3);

    let result = sem.try_acquire_many(5);
    assert!(matches!(result, Err(TryAcquireError::NoPermits)));
    assert_eq!(sem.available_permits(), 3); // No change
}

#[tokio::test]
async fn test_try_acquire_many_zero_permits() {
    let sem = RankedSemaphore::new_fifo(5);

    let permit = sem.try_acquire_many(0).unwrap();
    assert_eq!(permit.num_permits(), 0);
    assert_eq!(sem.available_permits(), 5); // No change
}

#[tokio::test]
async fn test_async_acquire_success() {
    let sem = RankedSemaphore::new_fifo(2);

    let permit1 = timeout(Duration::from_millis(100), sem.acquire())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sem.available_permits(), 1);

    let permit2 = timeout(Duration::from_millis(100), sem.acquire())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sem.available_permits(), 0);

    drop(permit1);
    drop(permit2);
}

#[tokio::test]
async fn test_async_acquire_many_success() {
    let sem = RankedSemaphore::new_fifo(5);

    let permit = timeout(Duration::from_millis(100), sem.acquire_many(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sem.available_permits(), 2);
    assert_eq!(permit.num_permits(), 3);

    drop(permit);
    assert_eq!(sem.available_permits(), 5);
}

#[tokio::test]
async fn test_semaphore_close_basic() {
    let sem = RankedSemaphore::new_fifo(5);
    assert!(!sem.is_closed());

    sem.close();
    assert!(sem.is_closed());

    // try_acquire should fail on closed semaphore
    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::Closed)));
}

#[tokio::test]
async fn test_semaphore_close_async_acquire() {
    let sem = Arc::new(RankedSemaphore::new_fifo(0)); // No permits available

    // Start an acquire operation that will wait
    let sem_for_task = Arc::clone(&sem);
    let acquire_task = tokio::spawn(async move { sem_for_task.acquire_owned().await });

    // Give some time for the acquire to register as waiting
    sleep(Duration::from_millis(10)).await;

    // Close the semaphore
    sem.close();

    // The acquire should fail with AcquireError
    let result = timeout(Duration::from_millis(100), acquire_task).await;
    assert!(result.is_ok());
    let acquire_result = result.unwrap().unwrap();
    assert!(matches!(acquire_result, Err(AcquireError { .. })));
}

#[tokio::test]
async fn test_try_acquire_closed_semaphore() {
    let sem = RankedSemaphore::new_fifo(5);
    sem.close();

    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::Closed)));

    let result_many = sem.try_acquire_many(2);
    assert!(matches!(result_many, Err(TryAcquireError::Closed)));
}

#[tokio::test]
async fn test_available_permits_consistency() {
    let sem = RankedSemaphore::new_fifo(10);

    // Test multiple acquisitions and releases
    let mut permits = Vec::new();

    for i in 1..=5 {
        let permit = sem.try_acquire().unwrap();
        permits.push(permit);
        assert_eq!(sem.available_permits(), 10 - i);
    }

    // Release permits one by one
    for i in 1..=5 {
        permits.pop();
        assert_eq!(sem.available_permits(), 5 + i);
    }
}

#[tokio::test]
async fn test_permit_drop_behavior() {
    let sem = RankedSemaphore::new_fifo(3);

    {
        let _permit1 = sem.try_acquire().unwrap();
        let _permit2 = sem.try_acquire().unwrap();
        assert_eq!(sem.available_permits(), 1);

        // Permits should be released when dropped
    }

    // After permits are dropped, all should be available
    assert_eq!(sem.available_permits(), 3);
}

#[tokio::test]
async fn test_error_display_formatting() {
    // Test AcquireError through actual closed semaphore
    let sem = RankedSemaphore::new_fifo(0);
    sem.close();
    let acquire_result = timeout(Duration::from_millis(50), sem.acquire()).await;
    if let Ok(Err(acquire_error)) = acquire_result {
        assert_eq!(format!("{acquire_error}"), "semaphore closed");
    }

    let try_acquire_closed = TryAcquireError::Closed;
    assert_eq!(format!("{try_acquire_closed}"), "semaphore closed");

    let try_acquire_no_permits = TryAcquireError::NoPermits;
    assert_eq!(
        format!("{try_acquire_no_permits}"),
        "no permits available"
    );
}

#[tokio::test]
async fn test_try_acquire_error_methods() {
    let closed_error = TryAcquireError::Closed;
    assert!(closed_error.is_closed());
    assert!(!closed_error.is_no_permits());

    let no_permits_error = TryAcquireError::NoPermits;
    assert!(!no_permits_error.is_closed());
    assert!(no_permits_error.is_no_permits());
}

#[tokio::test]
async fn test_edge_case_rapid_acquire_release() {
    let sem = RankedSemaphore::new_fifo(1);

    // Rapidly acquire and release
    for _ in 0..100 {
        let permit = sem.try_acquire().unwrap();
        assert_eq!(sem.available_permits(), 0);
        drop(permit);
        assert_eq!(sem.available_permits(), 1);
    }
}

#[tokio::test]
async fn test_edge_case_large_acquire_many() {
    let sem = RankedSemaphore::new_fifo(1000);

    let permit = sem.try_acquire_many(999).unwrap();
    assert_eq!(sem.available_permits(), 1);
    assert_eq!(permit.num_permits(), 999);

    drop(permit);
    assert_eq!(sem.available_permits(), 1000);
}

#[tokio::test]
async fn test_edge_case_single_permit_semaphore() {
    let sem = RankedSemaphore::new_fifo(1);

    let permit = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 0);

    // Should fail to acquire more
    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::NoPermits)));

    drop(permit);
    assert_eq!(sem.available_permits(), 1);
}

#[tokio::test]
async fn test_debug_formatting() {
    let sem = RankedSemaphore::new_fifo(5);
    let debug_str = format!("{sem:?}");
    assert!(debug_str.contains("RankedSemaphore"));

    let permit = sem.try_acquire().unwrap();
    let permit_debug = format!("{permit:?}");
    assert!(permit_debug.contains("RankedSemaphorePermit"));
}

#[tokio::test]
async fn test_send_sync_bounds() {
    // Test that RankedSemaphore implements Send + Sync
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RankedSemaphore>();

    // Test across thread boundary
    let sem = Arc::new(RankedSemaphore::new_fifo(1));
    let sem_clone = Arc::clone(&sem);

    let handle = tokio::spawn(async move {
        let _permit = sem_clone.try_acquire().unwrap();
    });

    handle.await.unwrap();
}

#[tokio::test]
async fn test_async_context_switching() {
    let sem = RankedSemaphore::new_fifo(1);

    let permit = sem.acquire().await.unwrap();

    // Yield to allow other tasks to run
    tokio::task::yield_now().await;

    assert_eq!(sem.available_permits(), 0);
    drop(permit);
    assert_eq!(sem.available_permits(), 1);
}
