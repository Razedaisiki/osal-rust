//! Runtime lifecycle contract tests for Mock backend.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit -- --test-threads=1
//! ```

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;
use osal_backend_mock::runtime;

// ---------------------------------------------------------------------------
// RuntimeFactory impl
// ---------------------------------------------------------------------------

struct MockRuntimeFactory;

impl osal_testkit::factory::RuntimeFactory for MockRuntimeFactory {
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
fn mock_runtime_core_contracts() {
    osal_testkit::contract::runtime::run_core_contracts::<MockRuntimeFactory>();
}
