//! Backend type aliases — resolve to the active backend's concrete types.
//!
//! Each type alias maps to the corresponding type from the selected
//! backend crate. Application code uses these aliases without knowing
//! which backend is active.

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::queue::MockQueue as Queue;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
compile_error!("backend-posix Queue is not yet implemented");
