//! Unit tests for PaaS core logic

use std::collections::HashMap;

use strata_paas::*;

#[test]
fn test_task_status_is_retriable() {
    // Only TransientFailure should be retriable
    assert!(TaskStatus::TransientFailure {
        retry_count: 1,
        error: "test".into()
    }
    .is_retriable());

    // Other states are not retriable (including Pending, which is handled separately)
    assert!(!TaskStatus::Pending.is_retriable());
    assert!(!TaskStatus::Queued.is_retriable());
    assert!(!TaskStatus::Proving.is_retriable());
    assert!(!TaskStatus::Completed.is_retriable());
    assert!(!TaskStatus::PermanentFailure {
        error: "test".into()
    }
    .is_retriable());
}

#[test]
fn test_retry_config_delay() {
    let config = RetryConfig {
        max_retries: 5,
        base_delay_secs: 1,
        multiplier: 2.0,
        max_delay_secs: 30,
    };

    // Test exponential backoff
    assert_eq!(config.calculate_delay(0), 1);
    assert_eq!(config.calculate_delay(1), 2);
    assert_eq!(config.calculate_delay(2), 4);
    assert_eq!(config.calculate_delay(3), 8);

    // Test max delay cap
    assert_eq!(config.calculate_delay(10), 30);

    // Test should_retry
    assert!(config.should_retry(0));
    assert!(config.should_retry(4));
    assert!(!config.should_retry(5));
}

#[test]
fn test_zkvm_backend_serialization() {
    // Test that ZkVmBackend can be serialized
    let backend = ZkVmBackend::Native;
    let json = serde_json::to_string(&backend).unwrap();
    assert!(json.contains("Native"));

    let backend = ZkVmBackend::SP1;
    let json = serde_json::to_string(&backend).unwrap();
    assert!(json.contains("SP1"));

    let backend = ZkVmBackend::Risc0;
    let json = serde_json::to_string(&backend).unwrap();
    assert!(json.contains("Risc0"));
}

#[test]
fn test_task_id_equality() {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    enum TestProgram {
        Program1,
        Program2,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestVariant {
        A,
        B,
    }

    impl ProgramType for TestProgram {
        type RoutingKey = TestVariant;

        fn routing_key(&self) -> Self::RoutingKey {
            match self {
                TestProgram::Program1 => TestVariant::A,
                TestProgram::Program2 => TestVariant::B,
            }
        }
    }

    let task1 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
    let task2 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
    let task3 = TaskId::new(TestProgram::Program2, ZkVmBackend::Native);
    let task4 = TaskId::new(TestProgram::Program1, ZkVmBackend::SP1);

    // Same program and backend should be equal
    assert_eq!(task1, task2);
    assert_eq!(task1.clone(), task2.clone());

    // Different program should not be equal
    assert_ne!(task1, task3);

    // Different backend should not be equal
    assert_ne!(task1, task4);
}

#[test]
fn test_task_id_hash() {
    use serde::{Deserialize, Serialize};
    use std::collections::HashSet;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    enum TestProgram {
        A,
        B,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestVariant {
        A,
        B,
    }

    impl ProgramType for TestProgram {
        type RoutingKey = TestVariant;

        fn routing_key(&self) -> Self::RoutingKey {
            match self {
                TestProgram::A => TestVariant::A,
                TestProgram::B => TestVariant::B,
            }
        }
    }

    let mut set = HashSet::new();

    let task1 = TaskId::new(TestProgram::A, ZkVmBackend::Native);
    let task2 = TaskId::new(TestProgram::A, ZkVmBackend::Native);

    // Should only store one copy
    set.insert(task1);
    set.insert(task2);
    assert_eq!(set.len(), 1);

    // Different task should be added
    set.insert(TaskId::new(TestProgram::B, ZkVmBackend::Native));
    assert_eq!(set.len(), 2);
}

#[test]
fn test_status_summary_serialization() {
    let summary = StatusSummary {
        total: 10,
        pending: 2,
        queued: 1,
        proving: 3,
        completed: 2,
        transient_failure: 1,
        permanent_failure: 1,
    };

    let json = serde_json::to_string(&summary).unwrap();
    let deserialized: StatusSummary = serde_json::from_str(&json).unwrap();

    assert_eq!(summary.total, deserialized.total);
    assert_eq!(summary.pending, deserialized.pending);
    assert_eq!(summary.queued, deserialized.queued);
    assert_eq!(summary.proving, deserialized.proving);
    assert_eq!(summary.completed, deserialized.completed);
    assert_eq!(summary.transient_failure, deserialized.transient_failure);
    assert_eq!(summary.permanent_failure, deserialized.permanent_failure);
}

#[test]
fn test_worker_config_creation() {
    let mut worker_count = HashMap::new();
    worker_count.insert(ZkVmBackend::Native, 4);
    worker_count.insert(ZkVmBackend::SP1, 2);

    let config = WorkerConfig {
        worker_count: worker_count.clone(),
        polling_interval_ms: 100,
    };

    assert_eq!(config.worker_count.get(&ZkVmBackend::Native), Some(&4));
    assert_eq!(config.worker_count.get(&ZkVmBackend::SP1), Some(&2));
    assert_eq!(config.worker_count.get(&ZkVmBackend::Risc0), None);
    assert_eq!(config.polling_interval_ms, 100);
}

#[test]
fn test_paas_config_creation() {
    let mut worker_count = HashMap::new();
    worker_count.insert(ZkVmBackend::Native, 2);

    let config = PaaSConfig {
        workers: WorkerConfig {
            worker_count,
            polling_interval_ms: 50,
        },
        retry: RetryConfig {
            max_retries: 3,
            base_delay_secs: 1,
            multiplier: 2.0,
            max_delay_secs: 60,
        },
    };

    assert_eq!(config.workers.polling_interval_ms, 50);
    assert_eq!(config.retry.max_retries, 3);
    assert_eq!(config.retry.base_delay_secs, 1);
}

#[test]
fn test_error_display() {
    let err = PaaSError::TaskNotFound("task_123".into());
    assert!(err.to_string().contains("task_123"));

    let err = PaaSError::TransientFailure("network error".into());
    assert!(err.to_string().contains("network error"));

    let err = PaaSError::PermanentFailure("invalid input".into());
    assert!(err.to_string().contains("invalid input"));

    let err = PaaSError::Config("missing config".into());
    assert!(err.to_string().contains("missing config"));
}

#[test]
fn test_task_status_display() {
    let status = TaskStatus::Pending;
    let debug = format!("{:?}", status);
    assert!(debug.contains("Pending"));

    let status = TaskStatus::TransientFailure {
        retry_count: 2,
        error: "timeout".into(),
    };
    let debug = format!("{:?}", status);
    assert!(debug.contains("TransientFailure"));
    assert!(debug.contains("2"));
    assert!(debug.contains("timeout"));
}
