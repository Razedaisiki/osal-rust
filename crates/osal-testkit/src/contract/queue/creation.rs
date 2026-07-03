//! Queue creation contract tests.

use osal_api::error::Error;
use osal_api::traits::queue::Queue as _;
use crate::factory::QueueFactory;

/// Valid creation returns correct metadata.
pub fn create_valid<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(8, 4).unwrap();
    assert_eq!(q.capacity(), 8);
    assert_eq!(q.msg_size(), 4);
    assert_eq!(q.len().unwrap(), 0);
    assert!(q.is_empty().unwrap());
    assert!(!q.is_full().unwrap());
}

/// Zero capacity returns InvalidParameter.
pub fn reject_zero_capacity<F: QueueFactory>(factory: &F) {
    let result = factory.create_queue(0, 4);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Zero message size returns InvalidParameter.
pub fn reject_zero_msg_size<F: QueueFactory>(factory: &F) {
    let result = factory.create_queue(8, 0);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Run all creation tests.
pub fn run<F: QueueFactory>(factory: &F) {
    create_valid::<F>(factory);
    reject_zero_capacity::<F>(factory);
    reject_zero_msg_size::<F>(factory);
}
