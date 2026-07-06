//! Backend type aliases — resolve to the active backend's concrete types.
//!
//! Each type alias maps to the corresponding type from the selected
//! backend crate. Application code uses these aliases without knowing
//! which backend is active.

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::queue::MockQueue as Queue;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::queue::PosixQueue as Queue;

// ---------------------------------------------------------------------------
// Mutex
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::mutex::MockMutex as Mutex;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::mutex::PosixMutexImpl as Mutex;
