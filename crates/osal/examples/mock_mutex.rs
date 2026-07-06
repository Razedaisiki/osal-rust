//! Mock mutex example — demonstrates basic lock/unlock and recursive locking.
//!
//! Run with:
//! ```bash
//! cargo run --example mock_mutex --no-default-features --features backend-mock
//! ```

use osal::prelude::*;

fn main() {
    let m = Mutex::new(0u32).unwrap();

    // Basic lock/unlock
    {
        let mut guard = m.lock(Timeout::NoWait).unwrap();
        *guard += 1;
        println!("Value: {}", *guard);
    }

    // Recursive lock
    {
        let g1 = m.lock(Timeout::NoWait).unwrap();
        let g2 = m.lock(Timeout::NoWait).unwrap();
        println!("Recursive lock: {} {}", *g1, *g2);
        drop(g2);
        drop(g1);
    }

    // Re-lock after all guards dropped
    let guard = m.lock(Timeout::Forever).unwrap();
    println!("Final value: {}", *guard);
}
