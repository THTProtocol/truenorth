//! Maximum step count enforcement.
//!
//! Tracks the number of steps executed for a task and returns
//! `ExecutionError::MaxStepsExceeded` when the limit is reached.

use truenorth_core::traits::execution::ExecutionError;
use uuid::Uuid;

/// Tracks step count and enforces a maximum.
///
/// Each `increment()` call increases the counter. When the counter
/// reaches `max_steps`, the next `increment()` returns an error.
#[derive(Debug)]
pub struct StepCounter {
    task_id: Uuid,
    max_steps: usize,
    current: usize,
}

impl StepCounter {
    /// Creates a new step counter for the given task.
    pub fn new(task_id: Uuid, max_steps: usize) -> Self {
        Self {
            task_id,
            max_steps,
            current: 0,
        }
    }

    /// Increments the step counter.
    ///
    /// Returns `Err(MaxStepsExceeded)` if the maximum has been reached.
    pub fn increment(&mut self) -> Result<usize, ExecutionError> {
        self.current += 1;
        if self.current > self.max_steps {
            return Err(ExecutionError::MaxStepsExceeded {
                task_id: self.task_id,
                max_steps: self.max_steps,
            });
        }
        Ok(self.current)
    }

    /// Returns the current step count.
    pub fn current(&self) -> usize {
        self.current
    }

    /// Returns the maximum allowed steps.
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Returns the number of remaining steps before the limit is hit.
    pub fn remaining(&self) -> usize {
        self.max_steps.saturating_sub(self.current)
    }

    /// Returns true if the maximum has been reached.
    pub fn is_exhausted(&self) -> bool {
        self.current >= self.max_steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_within_limit_succeeds() {
        let mut counter = StepCounter::new(Uuid::new_v4(), 5);
        for i in 1..=5 {
            assert_eq!(counter.increment().unwrap(), i);
        }
    }

    #[test]
    fn increment_beyond_limit_fails() {
        let task_id = Uuid::new_v4();
        let mut counter = StepCounter::new(task_id, 3);
        counter.increment().unwrap();
        counter.increment().unwrap();
        counter.increment().unwrap();
        let result = counter.increment();
        assert!(result.is_err());
        match result.unwrap_err() {
            ExecutionError::MaxStepsExceeded { task_id: tid, max_steps } => {
                assert_eq!(tid, task_id);
                assert_eq!(max_steps, 3);
            }
            _ => panic!("Expected MaxStepsExceeded"),
        }
    }

    #[test]
    fn is_exhausted_at_limit() {
        let mut counter = StepCounter::new(Uuid::new_v4(), 2);
        assert!(!counter.is_exhausted());
        counter.increment().unwrap();
        counter.increment().unwrap();
        assert!(counter.is_exhausted());
    }

    #[test]
    fn remaining_decrements() {
        let mut counter = StepCounter::new(Uuid::new_v4(), 10);
        assert_eq!(counter.remaining(), 10);
        counter.increment().unwrap();
        counter.increment().unwrap();
        assert_eq!(counter.remaining(), 8);
    }
}
