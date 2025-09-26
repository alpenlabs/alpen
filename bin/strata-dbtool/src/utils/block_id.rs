//! Utilities for parsing and validating block IDs

use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_primitives::{buf::Buf32, l1::L1BlockId, l2::L2BlockId};

/// Length of a hex-encoded block ID (32 bytes = 64 hex characters)
const HEX_BLOCK_ID_LENGTH: usize = 64;

/// Error message for invalid block ID format
const INVALID_BLOCK_ID_MSG: &str = "Block-id must be 32-byte / 64-char hex";

/// Parses a hex string into a 32-byte array
///
/// Accepts hex strings with or without "0x" prefix
/// Returns error if the hex string is not exactly 64 characters or contains invalid hex
pub(crate) fn parse_block_id_hex(hex_input: &str) -> Result<[u8; 32], DisplayedError> {
    let hex_str = hex_input.strip_prefix("0x").unwrap_or(hex_input);

    if hex_str.len() != HEX_BLOCK_ID_LENGTH {
        return Err(DisplayedError::UserError(
            INVALID_BLOCK_ID_MSG.into(),
            Box::new(hex_input.to_owned()),
        ));
    }

    <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))
}

/// Parses a hex string into an L2BlockId
///
/// # Arguments
/// * `hex_input` - Hex string with or without "0x" prefix
///
/// # Returns
/// * `Ok(L2BlockId)` - Successfully parsed block ID
/// * `Err(DisplayedError)` - Invalid hex format or length
///
/// # Examples
/// ```
/// let block_id = parse_l2_block_id("0x1234567890abcdef...")?;
/// let block_id = parse_l2_block_id("1234567890abcdef...")?;
/// ```
pub(crate) fn parse_l2_block_id(hex_input: &str) -> Result<L2BlockId, DisplayedError> {
    let bytes = parse_block_id_hex(hex_input)?;
    Ok(L2BlockId::from(Buf32::from(bytes)))
}

/// Parses a hex string into an L1BlockId
///
/// # Arguments
/// * `hex_input` - Hex string with or without "0x" prefix
///
/// # Returns
/// * `Ok(L1BlockId)` - Successfully parsed block ID
/// * `Err(DisplayedError)` - Invalid hex format or length
///
/// # Examples
/// ```
/// let block_id = parse_l1_block_id("0x1234567890abcdef...")?;
/// let block_id = parse_l1_block_id("1234567890abcdef...")?;
/// ```
pub(crate) fn parse_l1_block_id(hex_input: &str) -> Result<L1BlockId, DisplayedError> {
    let bytes = parse_block_id_hex(hex_input)?;
    Ok(L1BlockId::from(Buf32::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formats a block ID as a hex string with "0x" prefix
    fn format_block_id_hex(bytes: &[u8; 32]) -> String {
        format!("0x{}", hex::encode(bytes))
    }

    #[test]
    fn test_parse_block_id_hex_valid() {
        let valid_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = parse_block_id_hex(valid_hex);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_block_id_hex_with_prefix() {
        let valid_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = parse_block_id_hex(valid_hex);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_block_id_hex_invalid_length() {
        let invalid_hex = "1234567890abcdef"; // Too short
        let result = parse_block_id_hex(invalid_hex);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_block_id_hex_invalid_chars() {
        let invalid_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"; // Invalid 'g'
        let invalid_hex = &invalid_hex.replace('f', "g");
        let result = parse_block_id_hex(invalid_hex);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_l2_block_id() {
        let valid_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = parse_l2_block_id(valid_hex);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_l1_block_id() {
        let valid_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let result = parse_l1_block_id(valid_hex);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_block_id_hex() {
        let bytes = [
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78,
            0x90, 0xab, 0xcd, 0xef,
        ];
        let formatted = format_block_id_hex(&bytes);
        assert!(formatted.starts_with("0x"));
        assert_eq!(formatted.len(), 66); // 2 ("0x") + 64 (hex chars)
    }
}
