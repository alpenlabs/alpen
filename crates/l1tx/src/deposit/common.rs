use bitcoin::script::Instructions;
use strata_primitives::params::DepositTxParams;

use super::error::DepositParseError;
use crate::utils::next_bytes;

#[derive(Debug)]
pub struct DepositRequestScriptInfo {
    pub tap_ctrl_blk_hash: [u8; 32],
    pub ee_bytes: Vec<u8>,
}

/// check if magic bytes(unique set of bytes used to identify relevant tx) is present or not
pub fn check_magic_bytes(
    instructions: &mut Instructions<'_>,
    config: &DepositTxParams,
) -> Result<(), DepositParseError> {
    // magic bytes
    if let Some(magic_bytes) = next_bytes(instructions) {
        if magic_bytes != config.magic_bytes {
            return Err(DepositParseError::InvalidMagic);
        }
        return Ok(());
    }

    Err(DepositParseError::MissingMagic)
}

/// extracts the Execution environment bytes(most possibly EVM bytes)
pub fn extract_ee_bytes<'a>(
    instructions: &mut Instructions<'a>,
    config: &DepositTxParams,
) -> Result<&'a [u8], DepositParseError> {
    match next_bytes(instructions) {
        Some(ee_bytes) => {
            if ee_bytes.len() as u8 != config.address_length {
                return Err(DepositParseError::InvalidDestLen(ee_bytes.len() as u8));
            }
            Ok(ee_bytes)
        }
        None => Err(DepositParseError::MissingDest),
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        opcodes::all::OP_RETURN,
        script::{Builder, PushBytesBuf},
    };

    use super::*;
    use crate::deposit::{common::check_magic_bytes, test_utils::get_deposit_tx_config};

    #[test]
    fn test_check_magic_bytes_valid() {
        let config = get_deposit_tx_config();
        let script = Builder::new()
            .push_slice(PushBytesBuf::try_from(config.magic_bytes.clone()).unwrap())
            .push_opcode(OP_RETURN)
            .into_script();
        let mut instructions = script.instructions();

        let result = check_magic_bytes(&mut instructions, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_magic_bytes_invalid() {
        let config = get_deposit_tx_config();
        let script = Builder::new()
            .push_slice(
                PushBytesBuf::try_from("wrong_magic".to_string().as_bytes().to_vec()).unwrap(),
            )
            .push_opcode(OP_RETURN)
            .into_script();
        let mut instructions = script.instructions();

        let result = check_magic_bytes(&mut instructions, &config);
        assert!(matches!(result, Err(DepositParseError::InvalidMagic)));
    }

    #[test]
    fn test_check_magic_bytes_missing() {
        let config = get_deposit_tx_config();
        let script = Builder::new().push_opcode(OP_RETURN).into_script();
        let mut instructions = script.instructions();

        let result = check_magic_bytes(&mut instructions, &config);
        assert!(matches!(result, Err(DepositParseError::MissingMagic)));
    }

    #[test]
    fn test_extract_ee_bytes_valid() {
        let config = get_deposit_tx_config();
        let ee_bytes = vec![0; config.address_length as usize];
        let script = Builder::new()
            .push_slice(PushBytesBuf::try_from(ee_bytes.clone()).unwrap())
            .push_opcode(OP_RETURN)
            .into_script();
        let mut instructions = script.instructions();

        let result = extract_ee_bytes(&mut instructions, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ee_bytes);
    }

    #[test]
    fn test_extract_ee_bytes_invalid_length() {
        let config = get_deposit_tx_config();
        let ee_bytes = vec![0; (config.address_length as usize) + 1];
        let script = Builder::new()
            .push_slice(PushBytesBuf::try_from(ee_bytes.clone()).unwrap())
            .push_opcode(OP_RETURN)
            .into_script();
        let mut instructions = script.instructions();

        let result = extract_ee_bytes(&mut instructions, &config);
        assert!(matches!(result, Err(DepositParseError::InvalidDestLen(_))));
    }

    #[test]
    fn test_extract_ee_bytes_missing() {
        let config = get_deposit_tx_config();
        let script = Builder::new().push_opcode(OP_RETURN).into_script();
        let mut instructions = script.instructions();

        let result = extract_ee_bytes(&mut instructions, &config);
        assert!(matches!(result, Err(DepositParseError::MissingDest)));
    }
}
