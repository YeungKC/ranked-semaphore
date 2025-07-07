pub(crate) mod core;
pub(crate) mod futures;
pub(crate) mod methods;
pub(crate) mod permits;

// Re-export main types
pub use core::RankedSemaphore;
// Futures are not re-exported, they're returned by methods
pub use permits::{OwnedRankedSemaphorePermit, RankedSemaphorePermit};
