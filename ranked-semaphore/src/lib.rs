#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs, unreachable_pub, missing_debug_implementations)]
#![deny(rust_2018_idioms)]

//! # Ranked Semaphore - High-Performance Priority Semaphore
//!
//! A completely safe (zero unsafe) high-performance semaphore implementation with priority queuing.
//! Design goals: zero-cost abstractions, ultra-low latency, minimal memory footprint.
//!
//! ## Core Features
//!
//! - **High Performance**: Uncontended path <10ns latency
//! - **Zero unsafe**: 100% safe Rust code
//! - **Dynamic Priority**: Support for arbitrary priority levels
//! - **Zero Cost**: Memory allocation only for actively used priorities
//! - **Batch Optimized**: Smart batching reduces lock contention
//! - **Cache Friendly**: Optimized memory layout
//!
//! ## Usage Example
//!
//! ```rust
//! use ranked_semaphore::RankedSemaphore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a semaphore with 3 permits
//! let semaphore = RankedSemaphore::new_fifo(3);
//!
//! // Default priority acquisition
//! let _permit1 = semaphore.acquire().await?;
//!
//! // High priority acquisition
//! let _permit2 = semaphore.acquire_with_priority(10).await?;
//!
//! // Batch acquisition
//! let _permits = semaphore.acquire_many_with_priority(5, 2).await?;
//! # Ok(())
//! # }
//! ```

mod config;
mod error;
mod semaphore;
mod wait_queue;

pub use config::{PriorityConfig, QueueStrategy};
pub use error::{AcquireError, TryAcquireError};
pub use semaphore::{OwnedRankedSemaphorePermit, RankedSemaphore, RankedSemaphorePermit};

// Runtime-specific implementations are integrated into the main semaphore
// #[cfg(feature = "tokio")]
// #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
// pub mod tokio_impl;

// #[cfg(feature = "async-std")]
// #[cfg_attr(docsrs, doc(cfg(feature = "async-std")))]
// pub mod async_std_impl;

// #[cfg(feature = "smol")]
// #[cfg_attr(docsrs, doc(cfg(feature = "smol")))]
// pub mod smol_impl;

/// Prelude module for commonly used types.
pub mod prelude {
    pub use crate::{
        AcquireError, OwnedRankedSemaphorePermit, PriorityConfig, QueueStrategy, RankedSemaphore,
        RankedSemaphorePermit, TryAcquireError,
    };
}

pub use semaphore::{Acquire, AcquireOwned};
