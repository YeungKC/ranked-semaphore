//! Async-std runtime example for ranked-semaphore

use ranked_semaphore::RankedSemaphore;
use std::sync::Arc;
use std::time::Duration;

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Ranked Semaphore with Async-std Runtime ===\n");

    // Example 1: Basic usage
    basic_usage().await?;

    // Example 2: Concurrent tasks with priorities
    concurrent_tasks().await?;

    Ok(())
}

async fn basic_usage() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. Basic Usage");
    println!("--------------");

    // Create semaphore with Arc from the start to avoid borrowing issues
    let sem = Arc::new(RankedSemaphore::new_lifo(2));
    println!("Created LIFO semaphore with 2 permits");

    // Test immediate acquisition
    let permit1 = sem.try_acquire()?;
    println!(
        "Immediately acquired permit 1, available: {}",
        sem.available_permits()
    );

    let permit2 = sem.try_acquire()?;
    println!(
        "Immediately acquired permit 2, available: {}",
        sem.available_permits()
    );

    // Should fail now
    match sem.try_acquire() {
        Ok(_) => println!("Unexpected success!"),
        Err(e) => println!("Expected failure: {}", e),
    }

    // Test async acquisition
    let sem_clone = sem.clone();

    let task = async_std::task::spawn(async move {
        let _permit = sem_clone.acquire().await.unwrap();
        println!("Async task acquired permit");
        async_std::task::sleep(Duration::from_millis(100)).await;
        println!("Async task releasing permit");
    });

    // Release a permit to unblock the task
    async_std::task::sleep(Duration::from_millis(50)).await;
    drop(permit1);
    println!("Released permit 1");

    task.await;

    drop(permit2);
    println!(
        "Released permit 2, available: {}\n",
        sem.available_permits()
    );

    Ok(())
}

async fn concurrent_tasks() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. Concurrent Tasks with Priorities");
    println!("-----------------------------------");

    let sem = Arc::new(RankedSemaphore::new_fifo(1));

    // Block the semaphore
    let _blocker = sem.acquire().await?;
    println!("Semaphore blocked");

    // Start multiple tasks with different priorities
    let mut tasks = Vec::new();

    for (id, priority) in [(1, 0), (2, 5), (3, 1), (4, 10)] {
        let sem = sem.clone();
        tasks.push(async_std::task::spawn(async move {
            let start = std::time::Instant::now();
            let _permit = sem.acquire_with_priority(priority).await.unwrap();
            println!(
                "  Task {} (priority {}) started after {:?}",
                id,
                priority,
                start.elapsed()
            );

            // Simulate some work
            async_std::task::sleep(Duration::from_millis(50)).await;
            println!("  Task {} completed", id);
        }));

        // Small delay to ensure tasks queue in order
        async_std::task::sleep(Duration::from_millis(10)).await;
    }

    // Wait a bit then release the blocker
    async_std::task::sleep(Duration::from_millis(50)).await;
    println!("Releasing blocker...");
    drop(_blocker);

    // Wait for all tasks to complete
    for task in tasks {
        task.await;
    }

    println!("All concurrent tasks completed\n");
    Ok(())
}
