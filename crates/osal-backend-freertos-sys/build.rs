//! Build script for `osal-backend-freertos-sys`.
//!
//! Compiles the C shim (`osal_freertos_shim.c`) which is the only
//! compilation unit that `#include`s FreeRTOS headers.  On hosts
//! without a FreeRTOS toolchain, use `--features test-fixture` to
//! skip the build script; the fixture provides stub capability data.

fn main() {
    #[cfg(feature = "test-fixture")]
    {
        println!("cargo:warning=osal-backend-freertos-sys: using test fixture (no FreeRTOS kernel)");
        return;
    }

    #[cfg(not(feature = "test-fixture"))]
    {
        cc::Build::new()
            .file("csrc/osal_freertos_shim.c")
            .include("include")
            .compile("osal_freertos_shim");
    }
}
