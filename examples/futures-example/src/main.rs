//! Generic futures example for ranked-semaphore
//! This example shows how to use ranked-semaphore with any executor
//! that supports the standard Future trait.

use futures::executor::block_on;
use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Ranked Semaphore with Generic Futures ===\n");

    block_on(async {
        // Example 1: Basic synchronous usage
        basic_sync_usage()?;

        // Example 2: Basic async usage
        basic_async_usage().await?;

        // Example 3: Priority configuration
        priority_configuration().await?;

        Ok(())
    })
}

fn basic_sync_usage() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. Basic Synchronous Usage");
    println!("--------------------------");

    let sem = RankedSemaphore::new_fifo(5);
    println!("Created semaphore with 5 permits");

    // Try-acquire permits
    let permit1 = sem.try_acquire()?;
    println!(
        "Try-acquired permit 1, available: {}",
        sem.available_permits()
    );

    let permit2 = sem.try_acquire_many(3)?;
    println!(
        "Try-acquired 3 permits, available: {}",
        sem.available_permits()
    );

    let permit3 = sem.try_acquire()?;
    println!(
        "Try-acquired high priority permit, available: {}",
        sem.available_permits()
    );

    // Demonstrate permit operations
    println!("Permit 1 has {} permits", permit1.num_permits());
    println!("Permit 2 has {} permits", permit2.num_permits());

    // Add more permits
    sem.add_permits(2);
    println!("Added 2 permits, available: {}", sem.available_permits());

    // Release permits
    drop(permit1);
    drop(permit2);
    drop(permit3);
    println!(
        "Released all permits, available: {}\n",
        sem.available_permits()
    );

    Ok(())
}

async fn basic_async_usage() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. Basic Async Usage");
    println!("--------------------");

    let sem = Arc::new(RankedSemaphore::new_lifo(2));
    println!("Created LIFO semaphore with 2 permits");

    // Async acquire
    let permit1 = sem.clone().acquire_owned().await?;
    println!(
        "Async acquired owned permit, available: {}",
        sem.available_permits()
    );

    // Async acquire with priority
    let permit2 = sem.clone().acquire_owned_with_priority(5).await?;
    println!(
        "Async acquired priority permit, available: {}",
        sem.available_permits()
    );

    // Release permits
    drop(permit1);
    drop(permit2);
    println!(
        "Released all permits, available: {}\n",
        sem.available_permits()
    );

    Ok(())
}

async fn priority_configuration() -> Result<(), Box<dyn std::error::Error>> {
    println!("3. Priority Configuration");
    println!("-------------------------");

    // Create a complex priority configuration
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo) // Default: FIFO
        .exact(0, QueueStrategy::Fifo) // Priority 0: FIFO
        .less_or_equal(-1, QueueStrategy::Lifo) // Background tasks: LIFO
        .range(1, 4, QueueStrategy::Fifo) // Low priority: FIFO
        .greater_or_equal(5, QueueStrategy::Lifo); // High priority: LIFO

    let sem = RankedSemaphore::new_with_config(3, config);
    println!("Created semaphore with complex priority configuration");

    // Test different priority levels
    let priorities = [-5, 0, 1, 5, 10];

    for priority in priorities {
        let permit = sem.acquire_with_priority(priority).await?;
        let strategy = match priority {
            p if p <= -1 => "LIFO (range)",
            0 => "FIFO (exact)",
            1..=4 => "FIFO (range)",
            p if p >= 5 => "LIFO (>=5)",
            _ => "FIFO (default)",
        };
        println!(
            "Priority {}: {} - acquired successfully",
            priority, strategy
        );
        drop(permit);
    }

    // Test semaphore operations
    println!("Testing semaphore operations:");
    println!("  Available permits: {}", sem.available_permits());

    sem.forget_permits(1);
    println!("  Forgot 1 permits, available: {}", sem.available_permits());

    sem.close();
    println!("  Semaphore closed: {}", sem.is_closed());

    // Try to acquire on closed semaphore (should fail)
    match sem.try_acquire() {
        Ok(_) => println!("  Unexpected success on closed semaphore!"),
        Err(e) => println!("  Expected error on closed semaphore: {}", e),
    }

    println!("Priority configuration test completed\n");

    Ok(())
}
