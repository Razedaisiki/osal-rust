//! Build script for `osal-backend-freertos-sys`.
//!
//! Compiles the C shim (`osal_freertos_shim.c`) which is the only
//! compilation unit that `#include`s FreeRTOS headers.
//!
//! # Native build
//!
//! Requires three environment variables pointing to the application's
//! FreeRTOS source tree:
//!
//! - `OSAL_FREERTOS_KERNEL_INCLUDE` — path to `FreeRTOS.h` etc.
//! - `OSAL_FREERTOS_CONFIG_INCLUDE` — path to `FreeRTOSConfig.h`
//! - `OSAL_FREERTOS_PORT_INCLUDE`  — path to `portmacro.h` etc.
//!
//! # Test fixture
//!
//! `--features test-fixture` skips the build script entirely and
//! uses stub capability data (no FreeRTOS kernel required).

#[cfg(not(feature = "test-fixture"))]
use std::env;

fn main() {
    #[cfg(feature = "test-fixture")]
    {
        println!(
            "cargo:warning=osal-backend-freertos-sys: using test fixture (no FreeRTOS kernel)"
        );
    }

    #[cfg(not(feature = "test-fixture"))]
    {
        let kernel = env::var("OSAL_FREERTOS_KERNEL_INCLUDE")
            .expect("OSAL_FREERTOS_KERNEL_INCLUDE must be set (path to FreeRTOS.h)");
        let config = env::var("OSAL_FREERTOS_CONFIG_INCLUDE")
            .expect("OSAL_FREERTOS_CONFIG_INCLUDE must be set (path to FreeRTOSConfig.h)");
        let port = env::var("OSAL_FREERTOS_PORT_INCLUDE")
            .expect("OSAL_FREERTOS_PORT_INCLUDE must be set (path to portmacro.h)");

        println!("cargo:rerun-if-env-changed=OSAL_FREERTOS_KERNEL_INCLUDE");
        println!("cargo:rerun-if-env-changed=OSAL_FREERTOS_CONFIG_INCLUDE");
        println!("cargo:rerun-if-env-changed=OSAL_FREERTOS_PORT_INCLUDE");

        cc::Build::new()
            .file("csrc/osal_freertos_shim.c")
            .include("include")
            .include(kernel)
            .include(config)
            .include(port)
            .compile("osal_freertos_shim");
    }
}
