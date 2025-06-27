//! # ranked-semaphore
//!
//! **A priority-aware semaphore for async Rust.**
//!
//! ## Features
//! - Priority scheduling: Configurable priority-based task ordering
//! - No runtime dependency: Works with any async runtime (Tokio, async-std, smol, etc.)
//! - Flexible queue strategies: FIFO/LIFO and custom strategy
//!
//! ## Quick Start
//! ```rust
//! use ranked_semaphore::RankedSemaphore;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a semaphore with 3 permits, FIFO strategy
//!     let sem = RankedSemaphore::new_fifo(3);
//!
//!     // Acquire a permit (default priority)
//!     let permit = sem.acquire().await.unwrap();
//!
//!     // Acquire with custom priority
//!     let high = sem.acquire_with_priority(10).await.unwrap();
//!
//!     // Release permits automatically on drop
//!     drop(permit);
//!     drop(high);
//! }
//! ```
//!
//! ## Advanced Usage
//!
//! Use [`PriorityConfig`] and [`QueueStrategy`] for fine-grained control:
//!
//! ```rust
//! use ranked_semaphore::{RankedSemaphore, PriorityConfig, QueueStrategy};
//! use std::sync::Arc;
//!
//! let config = PriorityConfig::new()
//!     .default_strategy(QueueStrategy::Fifo)
//!     .exact(10, QueueStrategy::Lifo);
//!
//! let limiter = Arc::new(RankedSemaphore::new_with_config(2, config));
//!
//! let _admin = limiter.acquire_with_priority(10).await.unwrap();
//! let _guest = limiter.acquire_with_priority(0).await.unwrap();
//! ```
//!
//! See the [README](https://github.com/yeungkc/ranked-semaphore#readme) and [API docs](https://docs.rs/ranked-semaphore) for more details.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs, unreachable_pub, missing_debug_implementations)]
#![deny(rust_2018_idioms)]

mod config;
mod error;
mod semaphore;
mod wait_queue;

pub use config::{PriorityConfig, QueueStrategy};
pub use error::{AcquireError, TryAcquireError};
pub use semaphore::{OwnedRankedSemaphorePermit, RankedSemaphore, RankedSemaphorePermit};

pub use semaphore::{Acquire, AcquireOwned};
