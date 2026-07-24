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
pub use osal_backend_posix::queue::PosixQueue as Queue;

// ---------------------------------------------------------------------------
// Mutex
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::mutex::MockMutex as Mutex;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::mutex::PosixMutexImpl as Mutex;

#[cfg(all(
    feature = "backend-freertos",
    not(any(feature = "backend-posix", feature = "backend-mock"))
))]
pub use osal_backend_freertos::mutex::FreeRtosMutex as Mutex;

// ---------------------------------------------------------------------------
// Semaphore
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::semaphore::MockCountingSemaphore as CountingSemaphore;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::semaphore::PosixCountingSemaphore as CountingSemaphore;

#[cfg(all(
    feature = "backend-freertos",
    not(any(feature = "backend-posix", feature = "backend-mock"))
))]
pub use osal_backend_freertos::semaphore::FreeRtosCountingSemaphore as CountingSemaphore;

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::semaphore::MockBinarySemaphore as BinarySemaphore;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::semaphore::PosixBinarySemaphore as BinarySemaphore;

#[cfg(all(
    feature = "backend-freertos",
    not(any(feature = "backend-posix", feature = "backend-mock"))
))]
pub use osal_backend_freertos::semaphore::FreeRtosBinarySemaphore as BinarySemaphore;

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::clock::MockClock as Clock;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::clock::PosixClock as Clock;

#[cfg(all(
    feature = "backend-freertos",
    not(any(feature = "backend-posix", feature = "backend-mock"))
))]
pub use osal_backend_freertos::clock::FreeRtosClock as Clock;

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::timer::MockTimer as Timer;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::timer::PosixTimer as Timer;

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::system::MockSystem as System;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::system::PosixSystem as System;

#[cfg(all(
    feature = "backend-freertos",
    not(any(feature = "backend-posix", feature = "backend-mock"))
))]
pub use osal_backend_freertos::system::FreeRtosSystem as System;

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::task::MockTask as Task;

#[cfg(feature = "backend-mock")]
pub use osal_backend_mock::task::MockTaskBuilder as TaskBuilder;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::task::PosixTask as Task;

#[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
pub use osal_backend_posix::task::PosixTaskBuilder as TaskBuilder;
