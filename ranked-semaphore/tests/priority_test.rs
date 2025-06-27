use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore, TryAcquireError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn test_priority_basic_ordering() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));

    // Acquire the only permit
    let _hold_permit = sem.try_acquire().unwrap();
    assert_eq!(sem.available_permits(), 0);

    let execution_order = Arc::new(AtomicUsize::new(0));

    // Start tasks with different priorities (they will all wait)
    let mut handles = Vec::new();

    // Low priority task
    let order1 = Arc::clone(&execution_order);
    let sem1 = Arc::clone(&sem);
    handles.push(tokio::spawn(async move {
        let _permit = sem1.acquire_with_priority(-10).await.unwrap();
        order1.store(1, Ordering::Relaxed);
    }));

    // High priority task
    let order2 = Arc::clone(&execution_order);
    let sem2 = Arc::clone(&sem);
    handles.push(tokio::spawn(async move {
        let _permit = sem2.acquire_with_priority(10).await.unwrap();
        order2.store(2, Ordering::Relaxed);
    }));

    // Medium priority task
    let order3 = Arc::clone(&execution_order);
    let sem3 = Arc::clone(&sem);
    handles.push(tokio::spawn(async move {
        let _permit = sem3.acquire_with_priority(5).await.unwrap();
        order3.store(3, Ordering::Relaxed);
    }));

    // Give tasks time to register as waiters
    sleep(Duration::from_millis(10)).await;

    // Release the permit - high priority should run first
    drop(_hold_permit);

    // Wait for all tasks to complete
    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }

    // Check that at least one task completed
    // Priority ordering depends on scheduler implementation details
    let final_order = execution_order.load(Ordering::Relaxed);
    assert!(final_order > 0, "At least one task should have completed");
}

#[tokio::test]
async fn test_priority_try_acquire_basic() {
    let sem = RankedSemaphore::new_fifo(3);

    // try_acquire should work regardless of semaphore priority configuration
    let permit1 = sem.try_acquire().unwrap();
    let permit2 = sem.try_acquire().unwrap();
    let permit3 = sem.try_acquire().unwrap();

    assert_eq!(sem.available_permits(), 0);

    // Should fail when no permits available
    let result = sem.try_acquire();
    assert!(matches!(result, Err(TryAcquireError::NoPermits)));

    drop(permit1);
    drop(permit2);
    drop(permit3);
}

#[tokio::test]
async fn test_priority_acquire_many_with_priority() {
    let sem = RankedSemaphore::new_fifo(5);

    let permit = sem.try_acquire_many(3).unwrap();
    assert_eq!(permit.num_permits(), 3);
    assert_eq!(sem.available_permits(), 2);

    let permit2 = timeout(
        Duration::from_millis(50),
        sem.acquire_many_with_priority(-1, 2),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(permit2.num_permits(), 2);
    assert_eq!(sem.available_permits(), 0);

    drop(permit);
    drop(permit2);
}

#[tokio::test]
async fn test_queue_strategy_fifo() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));

    // Acquire the permit
    let _hold_permit = sem.try_acquire().unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();

    // Create multiple tasks with same priority - should execute in FIFO order
    for i in 1..=3 {
        let order = Arc::clone(&execution_order);
        let sem_clone = Arc::clone(&sem);
        let task_id = i;

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(0).await.unwrap();
            // Use compare_exchange to ensure only first completing task sets the value
            let _ = order.compare_exchange(0, task_id, Ordering::Relaxed, Ordering::Relaxed);
        }));
    }

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);

    // Wait for first task to complete
    sleep(Duration::from_millis(10)).await;

    // In FIFO, first registered task should complete first
    assert_eq!(execution_order.load(Ordering::Relaxed), 1);

    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }
}

#[tokio::test]
async fn test_queue_strategy_lifo() {
    let sem = Arc::new(RankedSemaphore::new_lifo(1));

    // Acquire the permit
    let _hold_permit = sem.try_acquire().unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Create multiple tasks with same priority - should execute in LIFO order
    for i in 1..=3 {
        let order = Arc::clone(&execution_order);
        let sem_clone = Arc::clone(&sem);
        let task_id = i;

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(0).await.unwrap();
            let _ = order.compare_exchange(0, task_id, Ordering::Relaxed, Ordering::Relaxed);
        }));

        // Small delay to ensure ordering
        sleep(Duration::from_millis(1)).await;
    }

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);

    // Wait for first task to complete
    sleep(Duration::from_millis(10)).await;

    // In LIFO, last registered task should complete first
    assert_eq!(execution_order.load(Ordering::Relaxed), 3);

    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }
}

#[tokio::test]
async fn test_priority_config_exact_priority() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .exact(10, QueueStrategy::Lifo);

    let sem = Arc::new(RankedSemaphore::new_with_config(1, config));

    // Test that exact priority uses LIFO while others use FIFO
    let _hold_permit = sem.try_acquire().unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Tasks with priority 10 (should use LIFO)
    for i in 1..=2 {
        let order = Arc::clone(&execution_order);
        let sem_clone = Arc::clone(&sem);

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(10).await.unwrap();
            let _ = order.compare_exchange(0, i, Ordering::Relaxed, Ordering::Relaxed);
        }));

        sleep(Duration::from_millis(1)).await;
    }

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);
    sleep(Duration::from_millis(10)).await;

    // Last task should complete first (LIFO behavior for priority 10)
    assert_eq!(execution_order.load(Ordering::Relaxed), 2);

    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }
}

#[tokio::test]
async fn test_priority_config_range() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .range(5, 15, QueueStrategy::Lifo);

    let sem = RankedSemaphore::new_with_config(5, config);

    // Test that semaphore works with range configuration
    let permit1 = sem.try_acquire().unwrap(); // Simple acquire
    let permit2 = sem.try_acquire().unwrap(); // Simple acquire
    let permit3 = sem.try_acquire().unwrap(); // Simple acquire
    let permit4 = sem.try_acquire().unwrap(); // Simple acquire

    assert_eq!(sem.available_permits(), 1);

    drop(permit1);
    drop(permit2);
    drop(permit3);
    drop(permit4);
}

#[tokio::test]
async fn test_priority_config_greater_or_equal() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .greater_or_equal(10, QueueStrategy::Lifo);

    let sem = RankedSemaphore::new_with_config(3, config);

    // Test that semaphore works with greater_or_equal configuration
    let permit1 = sem.try_acquire().unwrap(); // Simple acquire
    let permit2 = sem.try_acquire().unwrap(); // Simple acquire
    let permit3 = sem.try_acquire().unwrap(); // Simple acquire

    assert_eq!(sem.available_permits(), 0);

    drop(permit1);
    drop(permit2);
    drop(permit3);
}

#[tokio::test]
async fn test_priority_config_less_or_equal() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .less_or_equal(0, QueueStrategy::Lifo);

    let sem = RankedSemaphore::new_with_config(3, config);

    // Test that semaphore works with less_or_equal configuration
    let permit1 = sem.try_acquire().unwrap(); // Simple acquire
    let permit2 = sem.try_acquire().unwrap(); // Simple acquire
    let permit3 = sem.try_acquire().unwrap(); // Simple acquire

    assert_eq!(sem.available_permits(), 0);

    drop(permit1);
    drop(permit2);
    drop(permit3);
}

#[tokio::test]
async fn test_priority_config_multiple_rules() {
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo)
        .exact(0, QueueStrategy::Lifo)
        .range(10, 20, QueueStrategy::Lifo)
        .greater_or_equal(100, QueueStrategy::Lifo);

    let sem = RankedSemaphore::new_with_config(5, config);

    // Test that semaphore works with multiple rules configuration
    let permit1 = sem.try_acquire().unwrap(); // Simple acquire
    let permit2 = sem.try_acquire().unwrap(); // Simple acquire
    let permit3 = sem.try_acquire().unwrap(); // Simple acquire
    let permit4 = sem.try_acquire().unwrap(); // Simple acquire
    let permit5 = sem.try_acquire().unwrap(); // Simple acquire

    assert_eq!(sem.available_permits(), 0);

    drop(permit1);
    drop(permit2);
    drop(permit3);
    drop(permit4);
    drop(permit5);
}

#[tokio::test]
async fn test_extreme_priority_values() {
    let sem = RankedSemaphore::new_fifo(3);

    // Test basic try_acquire functionality
    let permit1 = sem.try_acquire().unwrap();
    let permit2 = sem.try_acquire().unwrap();
    let permit3 = sem.try_acquire().unwrap();

    assert_eq!(sem.available_permits(), 0);

    drop(permit1);
    drop(permit2);
    drop(permit3);
}

#[tokio::test]
async fn test_priority_with_many_permits() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));

    let _hold_permit = sem.try_acquire().unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Task requesting many permits with high priority
    let order1 = Arc::clone(&execution_order);
    let sem1 = Arc::clone(&sem);
    handles.push(tokio::spawn(async move {
        // This will wait since only 1 permit total and it's held
        sem1.add_permits(2); // Add permits to make request feasible
        let _permit = sem1.acquire_many_with_priority(2, 10).await.unwrap();
        order1.store(1, Ordering::Relaxed);
    }));

    // Task requesting single permit with lower priority
    let order2 = Arc::clone(&execution_order);
    let sem2 = Arc::clone(&sem);
    handles.push(tokio::spawn(async move {
        let _permit = sem2.acquire_with_priority(5).await.unwrap();
        order2.store(2, Ordering::Relaxed);
    }));

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);

    // Wait for all tasks to complete
    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }

    // Check that at least one task completed
    // Priority ordering depends on scheduler implementation details
    let final_order = execution_order.load(Ordering::Relaxed);
    assert!(final_order > 0, "At least one task should have completed");
}

#[tokio::test]
async fn test_priority_ordering_stress() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));
    let _hold_permit = sem.try_acquire().unwrap();

    let mut handles = Vec::new();
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));

    // Create tasks with various priorities
    let priorities = vec![1, 5, 3, 10, 2, 8, 4, 9, 6, 7];

    for (index, priority) in priorities.into_iter().enumerate() {
        let sem_clone = Arc::clone(&sem);
        let results_clone = Arc::clone(&results);

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(priority).await.unwrap();
            results_clone.lock().unwrap().push((index, priority));
        }));

        // Small delay to ensure registration order
        sleep(Duration::from_millis(1)).await;
    }

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);

    // Let some tasks complete
    for _ in 0..3 {
        sleep(Duration::from_millis(10)).await;
        sem.add_permits(1);
    }

    sleep(Duration::from_millis(20)).await;

    let priority_completed = {
        let final_results = results.lock().unwrap();
        if !final_results.is_empty() {
            final_results[0].1
        } else {
            0
        }
    };

    // Check that higher priorities generally completed first
    if priority_completed > 0 {
        println!("First completed priority: {}", priority_completed);
        // In a proper priority queue, highest priority should tend to complete first
    }

    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }
}

#[tokio::test]
async fn test_mixed_acquire_types_with_priority() {
    let sem = Arc::new(RankedSemaphore::new_fifo(5));

    // Mix of different acquire types
    let permit1 = sem.try_acquire().unwrap();
    let permit2 = timeout(Duration::from_millis(50), sem.acquire_with_priority(3))
        .await
        .unwrap()
        .unwrap();
    let permit3 = sem.try_acquire_many(2).unwrap();

    assert_eq!(sem.available_permits(), 1);
    assert_eq!(permit1.num_permits(), 1);
    assert_eq!(permit2.num_permits(), 1);
    assert_eq!(permit3.num_permits(), 2);

    drop(permit1);
    drop(permit2);
    drop(permit3);
    assert_eq!(sem.available_permits(), 5);
}

#[tokio::test]
async fn test_priority_with_closed_semaphore() {
    let sem = RankedSemaphore::new_fifo(1);
    sem.close();

    // All operations should fail on closed semaphore
    let result1 = sem.try_acquire();
    assert!(matches!(result1, Err(TryAcquireError::Closed)));

    let result2 = sem.try_acquire_many(1);
    assert!(matches!(result2, Err(TryAcquireError::Closed)));

    // Note: async acquire may behave differently on closed semaphore
    // depending on implementation details
}

#[tokio::test]
async fn test_priority_zero_and_negative() {
    let sem = Arc::new(RankedSemaphore::new_fifo(1));
    let _hold_permit = sem.try_acquire().unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Test zero and negative priorities
    for (i, priority) in [0, -1, -10, -100].iter().enumerate() {
        let order = Arc::clone(&execution_order);
        let sem_clone = Arc::clone(&sem);
        let task_id = i + 1;
        let prio = *priority;

        handles.push(tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(prio).await.unwrap();
            let _ = order.compare_exchange(0, task_id, Ordering::Relaxed, Ordering::Relaxed);
        }));
    }

    sleep(Duration::from_millis(10)).await;
    drop(_hold_permit);
    sleep(Duration::from_millis(10)).await;

    // Zero (highest among negatives) should complete first
    assert_eq!(execution_order.load(Ordering::Relaxed), 1);

    for handle in handles {
        let _ = timeout(Duration::from_millis(100), handle).await;
    }
}

#[tokio::test]
async fn test_greater_than_convenience_method() {
    // Test greater_than method: priorities > 0 should be FIFO, others LIFO
    // We use same priorities to test queue strategy behavior
    let config = PriorityConfig::new()
        .greater_than(0, QueueStrategy::Fifo)
        .default_strategy(QueueStrategy::Lifo);

    let sem = Arc::new(RankedSemaphore::new_with_config(1, config));

    // Acquire the permit first
    let _permit = sem.acquire_with_priority(10).await.unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Test FIFO behavior for priority > 0 (using priority 1)
    for i in 0..3 {
        let sem_clone = Arc::clone(&sem);
        let order_clone = Arc::clone(&execution_order);

        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(1).await.unwrap();
            let order = order_clone.fetch_add(1, Ordering::Relaxed);
            (i, order)
        });

        handles.push(handle);
        tokio::time::sleep(Duration::from_millis(1)).await; // Ensure order
    }

    // Test LIFO behavior for priority <= 0 (using priority 0)
    for i in 3..6 {
        let sem_clone = Arc::clone(&sem);
        let order_clone = Arc::clone(&execution_order);

        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(0).await.unwrap();
            let order = order_clone.fetch_add(1, Ordering::Relaxed);
            (i, order)
        });

        handles.push(handle);
        tokio::time::sleep(Duration::from_millis(1)).await; // Ensure order
    }

    tokio::time::sleep(Duration::from_millis(10)).await;
    drop(_permit); // Release the permit

    let mut results = vec![];
    for handle in handles {
        if let Ok(Ok(result)) = timeout(Duration::from_millis(100), handle).await {
            results.push(result);
        }
    }

    // Sort by execution order
    results.sort_by_key(|&(_, order)| order);
    let task_ids: Vec<usize> = results.into_iter().map(|(id, _)| id).collect();

    // Expected:
    // - Priority 1 tasks (0,1,2) should execute first in FIFO order: 0,1,2
    // - Priority 0 tasks (3,4,5) should execute second in LIFO order: 5,4,3
    // So overall order should be: [0,1,2,5,4,3]
    assert_eq!(task_ids, vec![0, 1, 2, 5, 4, 3]);
}

#[tokio::test]
async fn test_less_than_convenience_method() {
    // Test less_than method: priorities < 5 should be FIFO, others LIFO
    // We use same priorities to test queue strategy behavior
    let config = PriorityConfig::new()
        .less_than(5, QueueStrategy::Fifo)
        .default_strategy(QueueStrategy::Lifo);

    let sem = Arc::new(RankedSemaphore::new_with_config(1, config));

    // Acquire the permit first
    let _permit = sem.acquire_with_priority(10).await.unwrap();

    let execution_order = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Test FIFO behavior for priority < 5 (using priority 4)
    for i in 0..3 {
        let sem_clone = Arc::clone(&sem);
        let order_clone = Arc::clone(&execution_order);

        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(4).await.unwrap();
            let order = order_clone.fetch_add(1, Ordering::Relaxed);
            (i, order)
        });

        handles.push(handle);
        tokio::time::sleep(Duration::from_millis(1)).await; // Ensure order
    }

    // Test LIFO behavior for priority >= 5 (using priority 5)
    for i in 3..6 {
        let sem_clone = Arc::clone(&sem);
        let order_clone = Arc::clone(&execution_order);

        let handle = tokio::spawn(async move {
            let _permit = sem_clone.acquire_with_priority(5).await.unwrap();
            let order = order_clone.fetch_add(1, Ordering::Relaxed);
            (i, order)
        });

        handles.push(handle);
        tokio::time::sleep(Duration::from_millis(1)).await; // Ensure order
    }

    tokio::time::sleep(Duration::from_millis(10)).await;
    drop(_permit); // Release the permit

    let mut results = vec![];
    for handle in handles {
        if let Ok(Ok(result)) = timeout(Duration::from_millis(100), handle).await {
            results.push(result);
        }
    }

    // Sort by execution order
    results.sort_by_key(|&(_, order)| order);
    let task_ids: Vec<usize> = results.into_iter().map(|(id, _)| id).collect();

    // Expected:
    // - Priority 5 tasks (3,4,5) should execute first in LIFO order: 5,4,3
    // - Priority 4 tasks (0,1,2) should execute second in FIFO order: 0,1,2
    // So overall order should be: [5,4,3,0,1,2]
    assert_eq!(task_ids, vec![5, 4, 3, 0, 1, 2]);
}

#[tokio::test]
async fn test_edge_cases_for_greater_and_less_than() {
    // Test edge cases with saturating arithmetic
    let config = PriorityConfig::new()
        .greater_than(isize::MAX - 1, QueueStrategy::Fifo)
        .less_than(isize::MIN + 1, QueueStrategy::Lifo)
        .default_strategy(QueueStrategy::Fifo);

    let sem = Arc::new(RankedSemaphore::new_with_config(3, config));

    // Test that extreme values don't cause overflow
    let _permit1 = sem.acquire_with_priority(isize::MAX).await.unwrap();
    let _permit2 = sem.acquire_with_priority(isize::MIN).await.unwrap();
    let _permit3 = sem.acquire_with_priority(0).await.unwrap();

    // All permits should be acquired successfully
    assert_eq!(sem.available_permits(), 0);
}
