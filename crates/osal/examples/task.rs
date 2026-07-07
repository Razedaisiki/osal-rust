//! Task example — spawn and join with cached exit code.
//!
//! Works with any OSAL backend (Mock executes synchronously in spawn;
//! POSIX launches a real pthread):
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
    println!("Task exited with code: {}", exit.code());

    Ok(())
}
