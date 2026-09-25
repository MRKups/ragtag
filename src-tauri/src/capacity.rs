//! Command concurrency management and execution capacity limits.
//!
//! Provides isolated capacity pools for Ordinary (workload-heavy) commands
//! and Protected (cancellation, status, and health-check) commands to guarantee
//! that UI responsiveness and cancellation mechanisms remain operational even under
//! heavy ingestion or inference loads.

use domain::{AppError, CapacityState};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const ORDINARY_COMMAND_CAPACITY: usize = 16;
pub const PROTECTED_COMMAND_CAPACITY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPriority {
    Ordinary,
    Protected,
}

#[derive(Debug)]
pub struct CommandPermit {
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub struct ExecutionCapacity {
    ordinary_semaphore: Arc<Semaphore>,
    protected_semaphore: Arc<Semaphore>,
}

impl Default for ExecutionCapacity {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionCapacity {
    pub fn new() -> Self {
        Self {
            ordinary_semaphore: Arc::new(Semaphore::new(ORDINARY_COMMAND_CAPACITY)),
            protected_semaphore: Arc::new(Semaphore::new(PROTECTED_COMMAND_CAPACITY)),
        }
    }

    pub fn acquire(&self, priority: CommandPriority) -> Result<CommandPermit, AppError> {
        let semaphore = match priority {
            CommandPriority::Ordinary => &self.ordinary_semaphore,
            CommandPriority::Protected => &self.protected_semaphore,
        };

        match semaphore.clone().try_acquire_owned() {
            Ok(permit) => Ok(CommandPermit { _permit: permit }),
            Err(_) => {
                let (op, detail, rec) = match priority {
                    CommandPriority::Ordinary => (
                        "Acquire command capacity",
                        "The application command capacity is currently saturated by concurrent operations",
                        "Wait for active operations to complete before retrying.",
                    ),
                    CommandPriority::Protected => (
                        "Acquire protected command capacity",
                        "Protected interactive command capacity is temporarily exhausted",
                        "Retry the cancellation or status probe shortly.",
                    ),
                };
                Err(AppError::queue_full(op, detail, rec))
            }
        }
    }

    pub fn snapshot(&self) -> CapacityState {
        CapacityState {
            ordinary_available: self.ordinary_semaphore.available_permits(),
            ordinary_capacity: ORDINARY_COMMAND_CAPACITY,
            protected_available: self.protected_semaphore.available_permits(),
            protected_capacity: PROTECTED_COMMAND_CAPACITY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_reports_initial_snapshot() {
        let capacity = ExecutionCapacity::new();
        let snap = capacity.snapshot();
        assert_eq!(snap.ordinary_available, ORDINARY_COMMAND_CAPACITY);
        assert_eq!(snap.ordinary_capacity, ORDINARY_COMMAND_CAPACITY);
        assert_eq!(snap.protected_available, PROTECTED_COMMAND_CAPACITY);
        assert_eq!(snap.protected_capacity, PROTECTED_COMMAND_CAPACITY);
    }

    #[test]
    fn protects_capacity_when_ordinary_is_saturated() {
        let capacity = ExecutionCapacity::new();
        let mut ordinary_permits = Vec::new();
        for _ in 0..ORDINARY_COMMAND_CAPACITY {
            ordinary_permits.push(
                capacity
                    .acquire(CommandPriority::Ordinary)
                    .expect("ordinary permit within bounds"),
            );
        }

        let overflow = capacity.acquire(CommandPriority::Ordinary);
        assert!(overflow.is_err());
        assert_eq!(overflow.unwrap_err().code, domain::ErrorCode::QueueFull);

        let protected_permit = capacity.acquire(CommandPriority::Protected);
        assert!(protected_permit.is_ok());

        let snap = capacity.snapshot();
        assert_eq!(snap.ordinary_available, 0);
        assert_eq!(snap.protected_available, PROTECTED_COMMAND_CAPACITY - 1);

        drop(ordinary_permits.pop());
        assert!(capacity.acquire(CommandPriority::Ordinary).is_ok());
    }

    #[test]
    fn limits_protected_capacity() {
        let capacity = ExecutionCapacity::new();
        let mut protected_permits = Vec::new();
        for _ in 0..PROTECTED_COMMAND_CAPACITY {
            protected_permits.push(
                capacity
                    .acquire(CommandPriority::Protected)
                    .expect("protected permit within bounds"),
            );
        }

        let overflow = capacity.acquire(CommandPriority::Protected);
        assert!(overflow.is_err());
        assert_eq!(overflow.unwrap_err().code, domain::ErrorCode::QueueFull);
    }
}
