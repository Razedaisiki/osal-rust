//! Build script for the OSAL FreeRTOS QEMU MPS2 integration staticlib.
//!
//! In demo mode this resolves `OSAL_FREERTOS_DEMO` into a compile-time
//! selector, so a single `osal_demo_entry()` covers all seven demos.
//!
//! `rerun-if-env-changed` is essential: without it Cargo may reuse the
//! previously compiled selector when only `OSAL_FREERTOS_DEMO` changes
//! (e.g. `DEMO=mutex` → `DEMO=queue`).

use std::env;

const VALID_DEMOS: &[&str] = &[
    "mutex",
    "queue",
    "semaphore",
    "system",
    "task",
    "timer",
    "pipeline_demo",
];

fn main() {
    println!("cargo:rerun-if-env-changed=OSAL_FREERTOS_DEMO");

    if env::var_os("CARGO_FEATURE_DEMO").is_none() {
        return;
    }

    let demo = env::var("OSAL_FREERTOS_DEMO")
        .expect("OSAL_FREERTOS_DEMO must be set when the `demo` feature is enabled");

    if !VALID_DEMOS.contains(&demo.as_str()) {
        panic!("invalid OSAL_FREERTOS_DEMO: {demo}");
    }

    println!("cargo:rustc-env=OSAL_FREERTOS_DEMO={demo}");
}
