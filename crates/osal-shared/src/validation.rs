//! Common parameter validation for OSAL primitives.
//!
//! These functions encode the validation rules shared by all backends.
//! Using them prevents Mock, POSIX, and FreeRTOS from each inventing
//! different error semantics for the same invalid inputs.

use osal_api::error::{Error, Result};

/// Validate queue capacity.
///
/// Returns `Error::InvalidParameter` if `capacity == 0`.
pub fn validate_queue_capacity(capacity: usize) -> Result<()> {
    if capacity == 0 {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

/// Validate queue message size.
///
/// Returns `Error::InvalidParameter` if `message_size == 0`.
pub fn validate_queue_message_size(message_size: usize) -> Result<()> {
    if message_size == 0 {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

/// Validate that a send message has the correct length.
///
/// Returns `Error::InvalidMessageSize` if `actual != expected`.
pub fn validate_send_message_size(expected: usize, actual: usize) -> Result<()> {
    if actual != expected {
        return Err(Error::InvalidMessageSize);
    }
    Ok(())
}

/// Validate that a receive buffer is large enough.
///
/// Returns `Error::InvalidMessageSize` if `actual < expected`.
pub fn validate_recv_buffer_size(expected: usize, actual: usize) -> Result<()> {
    if actual < expected {
        return Err(Error::InvalidMessageSize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_zero_capacity() {
        assert!(matches!(
            validate_queue_capacity(0),
            Err(Error::InvalidParameter)
        ));
    }

    #[test]
    fn accept_positive_capacity() {
        assert!(validate_queue_capacity(4).is_ok());
    }

    #[test]
    fn reject_zero_message_size() {
        assert!(matches!(
            validate_queue_message_size(0),
            Err(Error::InvalidParameter)
        ));
    }

    #[test]
    fn accept_positive_message_size() {
        assert!(validate_queue_message_size(4).is_ok());
    }

    #[test]
    fn reject_send_wrong_size() {
        assert!(matches!(
            validate_send_message_size(4, 2),
            Err(Error::InvalidMessageSize)
        ));
    }

    #[test]
    fn accept_send_correct_size() {
        assert!(validate_send_message_size(4, 4).is_ok());
    }

    #[test]
    fn reject_recv_small_buffer() {
        assert!(matches!(
            validate_recv_buffer_size(4, 2),
            Err(Error::InvalidMessageSize)
        ));
    }

    #[test]
    fn accept_recv_large_buffer() {
        assert!(validate_recv_buffer_size(4, 8).is_ok());
    }
}
