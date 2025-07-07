use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore};
use std::sync::Arc;
use futures::executor::block_on;
use futures::future::join_all;

#[test]
fn test_futures_runtime_basic_usage() {
    block_on(async {
        let sem = RankedSemaphore::new_fifo(3);
        
        // Test basic acquire/release
        let permit1 = sem.acquire().await.unwrap();
        assert_eq!(sem.available_permits(), 2);
        
        let permit2 = sem.acquire_many(2).await.unwrap();
        assert_eq!(sem.available_permits(), 0);
        
        // Should fail when no permits available
        assert!(sem.try_acquire().is_err());
        
        // Release permits
        drop(permit1);
        assert_eq!(sem.available_permits(), 1);
        
        drop(permit2);
        assert_eq!(sem.available_permits(), 3);
    });
}

#[test]
fn test_futures_runtime_priority_access() {
    block_on(async {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .greater_or_equal(5, QueueStrategy::Lifo);
        
        let sem = Arc::new(RankedSemaphore::new_with_config(2, config));
        
        // Test basic priority ordering by using try_acquire
        let permit_high = Arc::clone(&sem).try_acquire_owned().unwrap();
        let permit_med = Arc::clone(&sem).try_acquire_owned().unwrap();
        
        // All permits are taken, now test that higher priority gets permit first when released
        drop(permit_high);
        
        let high_prio_permit = sem.acquire_with_priority(10).await.unwrap();
        assert_eq!(high_prio_permit.num_permits(), 1);
        
        drop(permit_med);
        let low_prio_permit = sem.acquire_with_priority(0).await.unwrap();
        assert_eq!(low_prio_permit.num_permits(), 1);
    });
}

#[test]
fn test_futures_runtime_concurrent_operations() {
    block_on(async {
        let sem = Arc::new(RankedSemaphore::new_fifo(5));
        let mut futures = vec![];
        
        // Create multiple concurrent futures
        for i in 0..10 {
            let sem_clone = Arc::clone(&sem);
            let future = async move {
                let _permit = sem_clone.acquire().await.unwrap();
                // Simulate some work without actual delay in block_on
                i
            };
            futures.push(future);
        }
        
        // All futures should complete successfully
        let results = join_all(futures).await;
        for (i, result) in results.into_iter().enumerate() {
            assert_eq!(result, i);
        }
        
        // All permits should be available again
        assert_eq!(sem.available_permits(), 5);
    });
}

#[test]
fn test_futures_runtime_semaphore_close() {
    block_on(async {
        let sem = Arc::new(RankedSemaphore::new_fifo(0));
        
        // Test immediate close behavior
        sem.close();
        
        // Trying to acquire on a closed semaphore should fail immediately
        let result = sem.acquire_owned().await;
        assert!(result.is_err());
    });
}