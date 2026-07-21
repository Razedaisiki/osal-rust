//! Runtime lifecycle contract tests for POSIX backend.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit -- --test-threads=1
//! ```

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;
use osal_backend_posix::runtime;

// ---------------------------------------------------------------------------
// RuntimeFactory impl
// ---------------------------------------------------------------------------

struct PosixRuntimeFactory;

impl osal_testkit::factory::RuntimeFactory for PosixRuntimeFactory {
    fn initialize() -> Result<()> {
        runtime::initialize()
    }

    fn shutdown() -> Result<()> {
        runtime::shutdown()
    }

    fn state() -> RuntimeState {
        runtime::state()
    }
}

// ---------------------------------------------------------------------------
// Contract entry points
// ---------------------------------------------------------------------------

#[test]
fn posix_runtime_core_contracts() {
    osal_testkit::contract::runtime::run_core_contracts::<PosixRuntimeFactory>();
}
