
use ranked_semaphore::{PriorityConfig, QueueStrategy, RankedSemaphore};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Ranked Semaphore with Tokio Runtime ===\n");

    // Example 1: Basic usage
    basic_usage().await?;

    // Example 2: Priority-based access
    priority_access().await?;

    // Example 3: Rate limiting simulation
    rate_limiting_simulation().await?;

    Ok(())
}

async fn basic_usage() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. Basic Usage");
    println!("--------------");

    let sem = RankedSemaphore::new_fifo(3);
    println!("Created FIFO semaphore with 3 permits");

    // Acquire permits
    let permit1 = sem.acquire().await?;
    println!("Acquired permit 1, available: {}", sem.available_permits());

    let permit2 = sem.acquire_many(2).await?;
    println!("Acquired 2 permits, available: {}", sem.available_permits());

    // Try to acquire more (should fail)
    match sem.try_acquire() {
        Ok(_) => println!("Unexpected success!"),
        Err(e) => println!("Expected failure: {}", e),
    }

    // Release permits
    drop(permit1);
    println!("Released 1 permit, available: {}", sem.available_permits());

    drop(permit2);
    println!(
        "Released 2 permits, available: {}\n",
        sem.available_permits()
    );

    Ok(())
}

async fn priority_access() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. Priority-based Access");
    println!("------------------------");

    let sem = Arc::new(RankedSemaphore::new_fifo(1));

    // Hold the only permit
    let _blocker = sem.acquire().await?;
    println!("Blocking permit acquired");

    // Start tasks with different priorities
    let mut handles = Vec::new();

    for (id, priority) in [(1, 0), (2, 10), (3, 5), (4, 15), (5, 1)] {
        let sem = sem.clone();
        handles.push(tokio::spawn(async move {
            let start = std::time::Instant::now();
            let _permit = sem.acquire_with_priority(priority).await.unwrap();
            println!(
                "  Task {} (priority {}) acquired permit after {:?}",
                id,
                priority,
                start.elapsed()
            );

            // Hold permit briefly
            tokio::time::sleep(Duration::from_millis(50)).await;
        }));
    }

    // Wait for tasks to queue up
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Release the blocking permit
    println!("Releasing blocking permit...");
    drop(_blocker);

    // Wait for all tasks
    for handle in handles {
        handle.await?;
    }

    println!("All priority tasks completed\n");
    Ok(())
}

async fn rate_limiting_simulation() -> Result<(), Box<dyn std::error::Error>> {
    println!("3. Rate Limiting Simulation");
    println!("---------------------------");

    // Simulate different user tiers with priority levels
    let config = PriorityConfig::new()
        .default_strategy(QueueStrategy::Fifo) // Guest users
        .exact(1, QueueStrategy::Fifo) // Regular users
        .greater_or_equal(5, QueueStrategy::Lifo) // Premium users (latest first)
        .exact(10, QueueStrategy::Fifo); // Admin users (fair ordering)

    let limiter = Arc::new(RankedSemaphore::new_with_config(2, config));

    println!("API server with 2 concurrent request slots");

    // Simulate API requests from different user types
    let requests = vec![
        ("Alice", 0),   // Guest
        ("Bob", 1),     // Regular
        ("Charlie", 7), // Premium
        ("David", 10),  // Admin
        ("Eve", 5),     // Premium
        ("Frank", 0),   // Guest
    ];

    let mut handles = Vec::new();

    for (user, priority) in requests {
        let limiter = limiter.clone();
        handles.push(tokio::spawn(async move {
            let start = std::time::Instant::now();
            let _permit = limiter.acquire_with_priority(priority).await.unwrap();

            let user_type = match priority {
                0 => "Guest",
                1 => "Regular",
                5..=9 => "Premium",
                10 => "Admin",
                _ => "Unknown",
            };

            println!(
                "  {} ({}) processing request (waited {:?})",
                user,
                user_type,
                start.elapsed()
            );

            // Simulate API processing time based on user tier
            let processing_time = match priority {
                10 => 100,    // Admin gets fast processing
                5..=9 => 150, // Premium gets medium processing
                _ => 200,     // Others get slower processing
            };

            tokio::time::sleep(Duration::from_millis(processing_time)).await;
            println!("  {} request completed", user);
        }));

        // Stagger request arrivals
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Wait for all requests
    for handle in handles {
        handle.await?;
    }

    println!("All API requests processed\n");
    Ok(())
}
