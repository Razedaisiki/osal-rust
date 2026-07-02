//! Mock queue roundtrip example.
//!
//! Demonstrates the OSAL facade API with the mock backend.
//!
//! ```bash
//! cargo run -p osal --example mock_queue \
//!   --no-default-features --features backend-mock
//! ```

use osal::prelude::*;

fn main() -> Result<()> {
    let q = Queue::new(4, 4)?;

    q.send(&[1, 2, 3, 4], Timeout::NoWait)?;
    q.send(&[5, 6, 7, 8], Timeout::NoWait)?;

    let mut out = [0u8; 4];
    q.recv(&mut out, Timeout::NoWait)?;
    assert_eq!(out, [1, 2, 3, 4]);

    q.recv(&mut out, Timeout::NoWait)?;
    assert_eq!(out, [5, 6, 7, 8]);

    println!("Mock queue roundtrip OK");
    Ok(())
}
