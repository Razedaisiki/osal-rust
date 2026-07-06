//! POSIX mutex example — demonstrates basic lock/unlock and timeout.
//!
//! Run with:
//! ```bash
//! cargo run --example posix_mutex
//! ```

use core::time::Duration;

use osal::prelude::*;

fn main() {
    let m = Mutex::new(0u32).unwrap();

    // Basic lock/unlock
    {
        let mut guard = m.lock(Timeout::NoWait).unwrap();
        *guard = 42;
        println!("Set value to: {}", *guard);
    }

    // Recursive lock
    {
        let g1 = m.lock(Timeout::NoWait).unwrap();
        let g2 = m.lock(Timeout::NoWait).unwrap();
        println!("Recursive: {} {}", *g1, *g2);
        drop(g2);
        drop(g1);
    }

    // Demonstrate After on already-held mutex — succeeds (recursive)
    let _guard = m.lock(Timeout::NoWait).unwrap();
    let result = m.lock(Timeout::After(Duration::from_millis(1)));
    assert!(result.is_ok());
    println!("Recursive After succeeded (same thread).");
    drop(_guard);

    println!("Mutex example complete.");
}
