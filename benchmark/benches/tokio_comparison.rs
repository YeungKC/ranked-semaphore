use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ranked_semaphore::RankedSemaphore;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore as TokioSemaphore;
use tokio::time::timeout;

// Same scales as quick_test: 1K, 5K, 10K, 50K, 100K
const TEST_SCALES: &[usize] = &[1_000, 5_000, 10_000, 50_000, 100_000];

/// Get test configuration - 10 samples minimum (Criterion requirement), fast timeouts
fn get_test_config(task_count: usize) -> (usize, Duration, Duration) {
    let sample_size = 100000; // Fixed sample size for all tests
    match task_count {
        1_000 => (sample_size, Duration::from_secs(3), Duration::from_secs(10)),
        5_000 => (sample_size, Duration::from_secs(3), Duration::from_secs(15)),
        10_000 => (sample_size, Duration::from_secs(5), Duration::from_secs(30)),
        50_000 => (
            sample_size,
            Duration::from_secs(15),
            Duration::from_secs(120),
        ),
        100_000 => (
            sample_size,
            Duration::from_secs(25),
            Duration::from_secs(180),
        ),
        _ => (
            sample_size,
            Duration::from_secs(10),
            Duration::from_secs(60),
        ),
    }
}

/// Main comparison test - exactly matching quick_test logic
fn bench_main_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("main_comparison");

    for &task_count in TEST_SCALES {
        let (sample_size, measurement_time, timeout_duration) = get_test_config(task_count);
        group.sample_size(sample_size);
        group.measurement_time(measurement_time);

        let permit_count = 10; // Fixed at 10 permits like quick_test
        let test_name = format!("{}k_tasks", task_count / 1000);

        // Tokio semaphore test
        group.bench_with_input(
            BenchmarkId::new("tokio", &test_name),
            &(task_count, permit_count),
            |b, &(task_count, permit_count)| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    rt.block_on(async {
                        let semaphore = Arc::new(TokioSemaphore::new(permit_count));
                        let start = Instant::now();

                        // Create tasks exactly like quick_test
                        let tasks: Vec<_> = (0..task_count)
                            .map(|_| {
                                let sem = Arc::clone(&semaphore);
                                tokio::spawn(async move {
                                    let _permit = sem.acquire().await.unwrap();
                                    tokio::task::yield_now().await;
                                })
                            })
                            .collect();

                        // Wait for completion with timeout
                        let result = timeout(timeout_duration, async {
                            for task in tasks {
                                task.await.unwrap();
                            }
                        })
                        .await;

                        let elapsed = start.elapsed();
                        black_box((result.is_ok(), elapsed));
                    });
                });
            },
        );

        // RankedSemaphore test - exactly like quick_test
        group.bench_with_input(
            BenchmarkId::new("ranked", &test_name),
            &(task_count, permit_count),
            |b, &(task_count, permit_count)| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    rt.block_on(async {
                        let semaphore = Arc::new(RankedSemaphore::new_fifo(permit_count));
                        let start = Instant::now();

                        // Create tasks with priority 1 exactly like quick_test
                        let tasks: Vec<_> = (0..task_count)
                            .map(|_| {
                                let sem = Arc::clone(&semaphore);
                                tokio::spawn(async move {
                                    let _permit = sem.acquire().await.unwrap();
                                    tokio::task::yield_now().await;
                                })
                            })
                            .collect();

                        // Wait for completion with timeout
                        let result = timeout(timeout_duration, async {
                            for task in tasks {
                                task.await.unwrap();
                            }
                        })
                        .await;

                        let elapsed = start.elapsed();
                        black_box((result.is_ok(), elapsed));
                    });
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_main_comparison);
criterion_main!(benches);
