[![Crates.io](https://img.shields.io/crates/v/ranked-semaphore.svg)](https://crates.io/crates/ranked-semaphore)
[![Docs.rs](https://docs.rs/ranked-semaphore/badge.svg)](https://docs.rs/ranked-semaphore)
[![CI](https://github.com/yeungkc/ranked-semaphore/actions/workflows/ci.yml/badge.svg)](https://github.com/yeungkc/ranked-semaphore/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Crates.io downloads](https://img.shields.io/crates/d/ranked-semaphore.svg)](https://crates.io/crates/ranked-semaphore)

# ranked-semaphore

> **A priority-aware semaphore for async Rust.**

---

## Features

- **Priority scheduling**: Configurable priority-based task ordering
- **No runtime dependency**: Works with any async runtime (Tokio, async-std, smol, etc.)
- **Flexible queue strategies**: FIFO/LIFO and custom strategy
- **High performance**: Optimized for low latency and high throughput

---

## Quick Start

```rust
use ranked_semaphore::RankedSemaphore;

#[tokio::main]
async fn main() {
    // Create a semaphore with 3 permits, FIFO strategy
    let sem = RankedSemaphore::new_fifo(3);

    // Acquire a permit (default priority)
    let permit = sem.acquire().await.unwrap();

    // Acquire with custom priority
    let high = sem.acquire_with_priority(10).await.unwrap();

    // Release permits automatically on drop
    drop(permit);
    drop(high);
}
```

---

## Advanced Usage

### Priority-based API Rate Limiting

```rust
use ranked_semaphore::{RankedSemaphore, PriorityConfig, QueueStrategy};
use std::sync::Arc;

let config = PriorityConfig::new()
    .default_strategy(QueueStrategy::Fifo)
    .exact(10, QueueStrategy::Lifo);

let limiter = Arc::new(RankedSemaphore::new_with_config(2, config));

let _admin = limiter.acquire_with_priority(10).await.unwrap();
let _guest = limiter.acquire_with_priority(0).await.unwrap();
```

---

## Runtime Support

The semaphore works with any async runtime. Runtime compatibility is ensured through comprehensive integration tests:

```sh
# Test Tokio runtime integration
cargo test --test runtime_tokio_test

# Test async-std runtime integration  
cargo test --test runtime_async_std_test

# Test smol runtime integration
cargo test --test runtime_smol_test

# Test futures runtime integration
cargo test --test runtime_futures_test
```

These tests serve as both verification and usage examples for each runtime.

---

## License

MIT

---

## Links

- [API Documentation (docs.rs)](https://docs.rs/ranked-semaphore)
- [Crates.io](https://crates.io/crates/ranked-semaphore)
- [docs.rs](https://docs.rs/ranked-semaphore)

---
