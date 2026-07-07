//! Minimal example demonstrating task spawn and join.
//!
//! Run with:
//! ```bash
//! cargo run -p osal --example task
//! cargo run -p osal --example task --no-default-features --features backend-mock
//! ```

use osal::prelude::*;

fn main() -> Result<()> {
    let task = TaskBuilder::new().name("worker").priority(1).spawn(|| {
        // worker body
    })?;

    let exit = task.join(Timeout::Forever)?;
    assert_eq!(exit, ExitCode::SUCCESS);
    println!("Task exited with code: {:?}", exit.code());

    Ok(())
}
