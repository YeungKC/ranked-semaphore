
use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore};
use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    smol::block_on(async {
        println!("=== Ranked Semaphore with Smol Runtime ===\n");

        // Example 1: Basic usage
        basic_usage().await?;

        // Example 2: Configuration-based queuing
        configuration_example().await?;

        Ok(())
    })
}

async fn basic_usage() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. Basic Usage");
    println!("--------------");

    // Create semaphore with Arc from the start
    let sem = Arc::new(RankedSemaphore::new_fifo(2));
    println!("Created FIFO semaphore with 2 permits");

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

    // Test async acquisition with shared semaphore
    let sem_clone = sem.clone();

    let task = smol::spawn(async move {
        let _permit = sem_clone.acquire().await.unwrap();
        println!("Async task acquired permit");
        smol::Timer::after(Duration::from_millis(100)).await;
        println!("Async task releasing permit");
    });

    // Release one permit to unblock the task
    smol::Timer::after(Duration::from_millis(50)).await;
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

async fn configuration_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. Configuration-based Queuing");
    println!("------------------------------");

    // Create a complex configuration
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo) // Default: FIFO
        .exact(0, QueueStrategy::Fifo) // Normal priority: FIFO
        .greater_or_equal(5, QueueStrategy::Lifo); // High priority: LIFO

    let sem = Arc::new(RankedSemaphore::new_with_config(1, config));
    println!("Created configured semaphore (FIFO for low priority, LIFO for high priority)");

    // Block the semaphore
    let _blocker = sem.acquire().await?;
    println!("Semaphore blocked with initial permit");

    // Queue tasks with different priorities
    let mut tasks = Vec::new();

    // Mix of priorities to demonstrate different queue behaviors
    for (id, priority, desc) in [
        (1, 0, "Normal"),
        (2, 10, "High"),
        (3, 1, "Low"),
        (4, 8, "High"),
        (5, 2, "Low"),
    ] {
        let sem = sem.clone();
        tasks.push(smol::spawn(async move {
            let start = std::time::Instant::now();
            let _permit = sem.acquire_with_priority(priority).await.unwrap();
            let queue_type = if priority >= 5 { "LIFO" } else { "FIFO" };
            println!(
                "  Task {} ({} priority {}, {} queue) executed after {:?}",
                id,
                desc,
                priority,
                queue_type,
                start.elapsed()
            );

            // Hold permit briefly
            smol::Timer::after(Duration::from_millis(30)).await;
        }));

        // Small delay to ensure queuing order
        smol::Timer::after(Duration::from_millis(10)).await;
    }

    // Wait for all tasks to queue up
    smol::Timer::after(Duration::from_millis(100)).await;

    // Release the blocker and watch execution order
    println!("Releasing blocker permit...");
    drop(_blocker);

    // Wait for all tasks
    for task in tasks {
        task.await;
    }

    println!("Configuration example completed\n");
    Ok(())
}
