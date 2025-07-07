use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ranked_semaphore::RankedSemaphore;
use std::sync::Arc;
use std::time::Duration;

/// Test the fast path optimization for common priorities vs overflow priorities
fn bench_priority_fast_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("priority_fast_path");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(3));
    
    let task_count = 1000;
    let permit_count = 10;
    
    // Test common priorities (within fast array range [-128, 127])
    let common_priorities = vec![0, 1, -1, 10, -10, 50, -50, 100, -100];
    
    // Test overflow priorities (outside fast array range)
    let overflow_priorities = vec![200, -200, 1000, -1000, 5000, -5000];
    
    for priority in common_priorities {
        group.bench_with_input(
            BenchmarkId::new("common_priority", priority),
            &(task_count, permit_count, priority),
            |b, &(task_count, permit_count, priority)| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    rt.block_on(async {
                        let semaphore = Arc::new(RankedSemaphore::new_fifo(permit_count));
                        
                        let tasks: Vec<_> = (0..task_count)
                            .map(|_| {
                                let sem = Arc::clone(&semaphore);
                                tokio::spawn(async move {
                                    let _permit = sem.acquire_with_priority(priority).await.unwrap();
                                    tokio::task::yield_now().await;
                                })
                            })
                            .collect();
                        
                        for task in tasks {
                            task.await.unwrap();
                        }
                        
                        black_box(());
                    });
                });
            },
        );
    }
    
    for priority in overflow_priorities {
        group.bench_with_input(
            BenchmarkId::new("overflow_priority", priority),
            &(task_count, permit_count, priority),
            |b, &(task_count, permit_count, priority)| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                b.iter(|| {
                    rt.block_on(async {
                        let semaphore = Arc::new(RankedSemaphore::new_fifo(permit_count));
                        
                        let tasks: Vec<_> = (0..task_count)
                            .map(|_| {
                                let sem = Arc::clone(&semaphore);
                                tokio::spawn(async move {
                                    let _permit = sem.acquire_with_priority(priority).await.unwrap();
                                    tokio::task::yield_now().await;
                                })
                            })
                            .collect();
                        
                        for task in tasks {
                            task.await.unwrap();
                        }
                        
                        black_box(());
                    });
                });
            },
        );
    }
    
    group.finish();
}

/// Test mixed priority workloads
fn bench_mixed_priority_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_priority_workload");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(3));
    
    let task_count = 1000;
    let permit_count = 10;
    
    // Test all common priorities (should use fast path)
    group.bench_with_input(
        BenchmarkId::new("all_common", "mixed"),
        &(task_count, permit_count),
        |b, &(task_count, permit_count)| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            b.iter(|| {
                rt.block_on(async {
                    let semaphore = Arc::new(RankedSemaphore::new_fifo(permit_count));
                    
                    let tasks: Vec<_> = (0..task_count)
                        .map(|i| {
                            let sem = Arc::clone(&semaphore);
                            let priority = (i % 256) as isize - 128; // All within fast range
                            tokio::spawn(async move {
                                let _permit = sem.acquire_with_priority(priority).await.unwrap();
                                tokio::task::yield_now().await;
                            })
                        })
                        .collect();
                    
                    for task in tasks {
                        task.await.unwrap();
                    }
                    
                    black_box(());
                });
            });
        },
    );
    
    // Test mixed common and overflow priorities
    group.bench_with_input(
        BenchmarkId::new("mixed_ranges", "mixed"),
        &(task_count, permit_count),
        |b, &(task_count, permit_count)| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            b.iter(|| {
                rt.block_on(async {
                    let semaphore = Arc::new(RankedSemaphore::new_fifo(permit_count));
                    
                    let tasks: Vec<_> = (0..task_count)
                        .map(|i| {
                            let sem = Arc::clone(&semaphore);
                            let priority = if i % 2 == 0 {
                                (i % 256) as isize - 128 // Fast range
                            } else {
                                (i % 1000) as isize + 1000 // Overflow range
                            };
                            tokio::spawn(async move {
                                let _permit = sem.acquire_with_priority(priority).await.unwrap();
                                tokio::task::yield_now().await;
                            })
                        })
                        .collect();
                    
                    for task in tasks {
                        task.await.unwrap();
                    }
                    
                    black_box(());
                });
            });
        },
    );
    
    group.finish();
}

criterion_group!(benches, bench_priority_fast_path, bench_mixed_priority_workload);
criterion_main!(benches);