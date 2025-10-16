use anyhow::anyhow;
use bitcoin::{
    opcodes::all::OP_PUSHNUM_1,
    script::{Instruction, Instructions},
    secp256k1::SECP256K1,
    taproot::TaprootBuilder,
    Address, Network, Opcode, XOnlyPublicKey,
};
use strata_crypto::multisig::aggregate_schnorr_keys;
use strata_params::{OperatorConfig, RollupParams};
use strata_primitives::{buf::Buf32, l1::BitcoinAddress};

/// Extract next instruction and try to parse it as an opcode
pub fn next_op(instructions: &mut Instructions<'_>) -> Option<Opcode> {
    let nxt = instructions.next();
    match nxt {
        Some(Ok(Instruction::Op(op))) => Some(op),
        _ => None,
    }
}

/// Extract next instruction and try to parse it as a byte slice
pub fn next_bytes<'a>(instructions: &mut Instructions<'a>) -> Option<&'a [u8]> {
    let ins = instructions.next();
    match ins {
        Some(Ok(Instruction::PushBytes(bytes))) => Some(bytes.as_bytes()),
        _ => None,
    }
}

/// Extract next integer value(unsigned)
pub fn next_u32(instructions: &mut Instructions<'_>) -> Option<u32> {
    let n = instructions.next();
    match n {
        Some(Ok(Instruction::PushBytes(bytes))) => {
            // Convert the bytes to an integer
            if bytes.len() != 4 {
                return None;
            }
            let mut buf = [0; 4];
            buf[..bytes.len()].copy_from_slice(bytes.as_bytes());
            Some(u32::from_be_bytes(buf))
        }
        Some(Ok(Instruction::Op(op))) => {
            // Handle small integers pushed by OP_1 to OP_16
            let opval = op.to_u8();
            let diff = opval - OP_PUSHNUM_1.to_u8();
            if (0..16).contains(&diff) {
                Some(diff as u32 + 1)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Returns taproot address along with untweaked internal pubkey
pub fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> anyhow::Result<(BitcoinAddress, XOnlyPublicKey)> {
    let x_only_pub_key = aggregate_schnorr_keys(operator_wallet_pks.iter())?;

    let taproot_builder = TaprootBuilder::new();
    let spend_info = taproot_builder
        .finalize(SECP256K1, x_only_pub_key)
        .map_err(|_| anyhow!("taproot finalization"))?;
    let merkle_root = spend_info.merkle_root();

    let addr = Address::p2tr(SECP256K1, x_only_pub_key, merkle_root, network);
    let addr = BitcoinAddress::parse(&addr.to_string(), network)?;

    Ok((addr, x_only_pub_key))
}

/// Reads the operator wallet public keys from Rollup params. Returns None if
/// not yet bootstrapped
// FIXME: This is only for devnet as these pks have to be read from the chain state
pub fn get_operator_wallet_pks(params: &RollupParams) -> Vec<Buf32> {
    let OperatorConfig::Static(operator_table) = &params.operator_config;

    operator_table.iter().map(|op| *op.wallet_pk()).collect()
}

#[cfg(any(test, feature = "test_utils"))]
pub mod test_utils {
    use bitcoin::{
        secp256k1::{Keypair, Secp256k1, SecretKey},
        Address, Network,
    };
    use strata_bridge_types::DepositEntry;
    use strata_params::Params;
    use strata_primitives::{
        l1::{BitcoinAddress, XOnlyPk},
        sorted_vec::FlatTable,
    };

    use crate::{filter::types::conv_deposit_to_fulfillment, TxFilterConfig};

    pub fn get_taproot_addr_and_keypair() -> (Address, Keypair) {
        // Generate valid signature
        let secp = Secp256k1::new();

        // Step 1. Create a random internal key (you can use a fixed one in tests)
        let secret_key = SecretKey::from_slice(&[42u8; 32]).unwrap();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        let (internal_xonly, _parity) = keypair.x_only_public_key();

        // Step 2. Create a Taproot address
        let taproot_addr = Address::p2tr(&secp, internal_xonly, None, Network::Regtest);
        (taproot_addr, keypair)
    }

    /// Helper function to create filter config
    pub fn create_tx_filter_config(params: &Params) -> (TxFilterConfig, Keypair) {
        let mut txconfig =
            TxFilterConfig::derive_from(params.rollup()).expect("can't get filter config");

        let mut deposit_config = txconfig.deposit_config.clone();
        let (taproot_addr, keypair) = get_taproot_addr_and_keypair();
        let (op_key, _) = keypair.x_only_public_key();
        deposit_config.operators_pubkey = XOnlyPk::new(op_key.serialize().into()).unwrap();
        deposit_config.address =
            BitcoinAddress::from_bytes(taproot_addr.script_pubkey().as_bytes(), Network::Regtest)
                .unwrap();

        txconfig.deposit_config = deposit_config;
        (txconfig, keypair)
    }

    /// Helper function to create filter config. Returns the filter config with expected withdrawal
    /// fulfillments derived from deposit entries.
    pub fn get_filter_config_from_deposit_entries(
        params: Params,
        deps: &[DepositEntry],
    ) -> TxFilterConfig {
        let mut filterconfig = TxFilterConfig::derive_from(params.rollup()).unwrap();
        // Watch all withdrawals that have been ordered.
        let exp_fulfillments = deps
            .iter()
            .flat_map(conv_deposit_to_fulfillment)
            .collect::<Vec<_>>();
        filterconfig.expected_withdrawal_fulfillments =
            FlatTable::try_from_unsorted(exp_fulfillments).expect("types: malformed deposits");
        filterconfig
    }
}
