/// Queue strategy for waiters at a given priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStrategy {
    /// First In, First Out - waiters are served in the order they arrive.
    Fifo,
    /// Last In, First Out - waiters are served in reverse order of arrival.
    Lifo,
}

/// A rule for determining queue strategy based on priority.
#[derive(Debug, Clone)]
pub(crate) enum PriorityRule {
    /// Default strategy for all priorities not matched by other rules.
    Default(QueueStrategy),
    /// Strategy for an exact priority value.
    Exact(isize, QueueStrategy),
    /// Strategy for priorities within a range [min, max] (inclusive).
    /// Use Range(x, isize::MAX) for greater-or-equal behavior.
    /// Use Range(isize::MIN, x) for less-or-equal behavior.
    Range(isize, isize, QueueStrategy),
}

/// Configuration for priority-based queue strategies.
#[derive(Debug, Clone)]
pub struct PriorityConfig {
    pub(crate) rules: Vec<PriorityRule>,
}

impl PriorityConfig {
    /// Create a new empty priority configuration.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Set the default queue strategy for all priorities not matched by other rules.
    pub fn default_strategy(mut self, strategy: QueueStrategy) -> Self {
        self.rules.push(PriorityRule::Default(strategy));
        self
    }

    /// Set the queue strategy for an exact priority value.
    pub fn exact(mut self, priority: isize, strategy: QueueStrategy) -> Self {
        self.rules.push(PriorityRule::Exact(priority, strategy));
        self
    }

    /// Set the queue strategy for priorities greater than or equal to the threshold.
    /// This is a convenience method that creates a range [threshold, isize::MAX].
    pub fn greater_or_equal(mut self, threshold: isize, strategy: QueueStrategy) -> Self {
        self.rules
            .push(PriorityRule::Range(threshold, isize::MAX, strategy));
        self
    }

    /// Set the queue strategy for priorities greater than the threshold (exclusive).
    /// This is a convenience method that creates a range [threshold + 1, isize::MAX].
    pub fn greater_than(mut self, threshold: isize, strategy: QueueStrategy) -> Self {
        let min = threshold.saturating_add(1);
        self.rules
            .push(PriorityRule::Range(min, isize::MAX, strategy));
        self
    }

    /// Set the queue strategy for priorities less than or equal to the threshold.
    /// This is a convenience method that creates a range [isize::MIN, threshold].
    pub fn less_or_equal(mut self, threshold: isize, strategy: QueueStrategy) -> Self {
        self.rules
            .push(PriorityRule::Range(isize::MIN, threshold, strategy));
        self
    }

    /// Set the queue strategy for priorities less than the threshold (exclusive).
    /// This is a convenience method that creates a range [isize::MIN, threshold - 1].
    pub fn less_than(mut self, threshold: isize, strategy: QueueStrategy) -> Self {
        let max = threshold.saturating_sub(1);
        self.rules
            .push(PriorityRule::Range(isize::MIN, max, strategy));
        self
    }

    /// Set the queue strategy for priorities within a range [min, max] (inclusive).
    pub fn range(mut self, min: isize, max: isize, strategy: QueueStrategy) -> Self {
        self.rules.push(PriorityRule::Range(min, max, strategy));
        self
    }

    /// Resolve the queue strategy for a given priority.
    ///
    /// Rules are evaluated in the following order:
    /// 1. Exact matches (highest priority)
    /// 2. Range matches (first match wins)
    /// 3. Default strategy
    pub(crate) fn resolve_strategy(&self, priority: isize) -> QueueStrategy {
        let mut default_strategy = QueueStrategy::Fifo;

        // First pass: Check for exact matches
        for rule in &self.rules {
            match rule {
                PriorityRule::Exact(p, strategy) if *p == priority => return *strategy,
                PriorityRule::Default(strategy) => default_strategy = *strategy,
                _ => {}
            }
        }

        // Second pass: Check for range matches
        for rule in &self.rules {
            match rule {
                PriorityRule::Range(min, max, strategy) if priority >= *min && priority <= *max => {
                    return *strategy
                }
                _ => {}
            }
        }

        default_strategy
    }

    /// Returns the queue strategy that would be used for the given priority.
    ///
    /// This method evaluates all configured rules to determine which queue strategy
    /// (FIFO or LIFO) would be applied to a waiter with the specified priority.
    ///
    /// # Arguments
    ///
    /// * `priority` - The priority level to query
    ///
    /// # Returns
    ///
    /// The [`QueueStrategy`] that would be used for the given priority.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ranked_semaphore::{PriorityConfig, QueueStrategy};
    ///
    /// let config = PriorityConfig::new()
    ///     .default_strategy(QueueStrategy::Fifo)
    ///     .exact(10, QueueStrategy::Lifo);
    ///
    /// assert_eq!(config.strategy_for_priority(5), QueueStrategy::Fifo);
    /// assert_eq!(config.strategy_for_priority(10), QueueStrategy::Lifo);
    /// ```
    pub fn strategy_for_priority(&self, priority: isize) -> QueueStrategy {
        self.resolve_strategy(priority)
    }
}

impl Default for PriorityConfig {
    fn default() -> Self {
        Self::new().default_strategy(QueueStrategy::Fifo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .exact(5, QueueStrategy::Lifo);

        assert_eq!(config.resolve_strategy(5), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(4), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(6), QueueStrategy::Fifo);
    }

    #[test]
    fn test_range_match() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .range(1, 10, QueueStrategy::Lifo);

        assert_eq!(config.resolve_strategy(0), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(1), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(5), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(10), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(11), QueueStrategy::Fifo);
    }

    #[test]
    fn test_greater_or_equal() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .greater_or_equal(5, QueueStrategy::Lifo);

        assert_eq!(config.resolve_strategy(4), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(5), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(10), QueueStrategy::Lifo);
    }

    #[test]
    fn test_greater_than() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .greater_than(5, QueueStrategy::Lifo);

        assert_eq!(config.resolve_strategy(5), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(6), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(10), QueueStrategy::Lifo);
    }

    #[test]
    fn test_less_or_equal() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Lifo)
            .less_or_equal(0, QueueStrategy::Fifo);

        assert_eq!(config.resolve_strategy(-5), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(0), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(1), QueueStrategy::Lifo);
    }

    #[test]
    fn test_less_than() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .less_than(0, QueueStrategy::Lifo);

        assert_eq!(config.resolve_strategy(-5), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(0), QueueStrategy::Fifo);
        assert_eq!(config.resolve_strategy(1), QueueStrategy::Fifo);
    }

    #[test]
    fn test_priority_order() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .greater_or_equal(5, QueueStrategy::Lifo)
            .exact(10, QueueStrategy::Fifo); // Exact should override range

        assert_eq!(config.resolve_strategy(5), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(8), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(10), QueueStrategy::Fifo); // Exact wins
    }

    #[test]
    fn test_multiple_thresholds() {
        let config = PriorityConfig::new()
            .default_strategy(QueueStrategy::Fifo)
            .range(5, isize::MAX, QueueStrategy::Lifo)
            .range(10, isize::MAX, QueueStrategy::Fifo); // More specific range should win

        assert_eq!(config.resolve_strategy(4), QueueStrategy::Fifo);
        // First range [5, MAX] matches, so LIFO (first match wins)
        assert_eq!(config.resolve_strategy(7), QueueStrategy::Lifo);
        assert_eq!(config.resolve_strategy(15), QueueStrategy::Lifo);
    }
}
