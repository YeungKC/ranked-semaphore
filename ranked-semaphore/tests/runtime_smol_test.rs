use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn test_smol_runtime_basic_usage() {
    smol::block_on(async {
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
fn test_smol_runtime_priority_access() {
    smol::block_on(async {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .greater_or_equal(5, QueueStrategy::Lifo);

        let sem = Arc::new(RankedSemaphore::new_with_config(1, config));

        // Hold the permit initially
        let _hold_permit = sem.acquire_with_priority(10).await.unwrap();

        let mut handles = vec![];
        let results: Arc<smol::lock::Mutex<Vec<isize>>> =
            Arc::new(smol::lock::Mutex::new(Vec::new()));

        // Spawn tasks with different priorities
        for priority in [0, 5, 10] {
            let sem_clone = Arc::clone(&sem);
            let results_clone = Arc::clone(&results);

            let handle = smol::spawn(async move {
                let _permit = sem_clone.acquire_with_priority(priority).await.unwrap();
                results_clone.lock().await.push(priority);
            });

            handles.push(handle);
            smol::Timer::after(Duration::from_millis(10)).await;
        }

        // Release the initial permit to start the queue
        drop(_hold_permit);

        // Wait for all tasks to complete
        for handle in handles {
            handle.await;
        }

        let results = results.lock().await;
        // Higher priorities should complete first
        assert_eq!(results[0], 10);
    });
}

#[test]
fn test_smol_runtime_concurrent_operations() {
    smol::block_on(async {
        let sem = Arc::new(RankedSemaphore::new_fifo(5));
        let mut handles = vec![];

        // Spawn multiple concurrent tasks
        for i in 0..10 {
            let sem_clone = Arc::clone(&sem);
            let handle = smol::spawn(async move {
                let _permit = sem_clone.acquire().await.unwrap();
                smol::Timer::after(Duration::from_millis(50)).await;
                i
            });
            handles.push(handle);
        }

        // All tasks should complete successfully
        for (i, handle) in handles.into_iter().enumerate() {
            let result = handle.await;
            assert_eq!(result, i);
        }

        // All permits should be available again
        assert_eq!(sem.available_permits(), 5);
    });
}

#[test]
fn test_smol_runtime_semaphore_close() {
    smol::block_on(async {
        let sem = Arc::new(RankedSemaphore::new_fifo(0));
        let sem_clone = Arc::clone(&sem);

        // Spawn a task that will wait
        let handle = smol::spawn(async move { sem_clone.acquire_owned().await });

        // Give the task time to start waiting
        smol::Timer::after(Duration::from_millis(10)).await;

        // Close the semaphore
        sem.close();

        // The waiting task should receive an error
        let result = handle.await;
        assert!(result.is_err());
    });
}
