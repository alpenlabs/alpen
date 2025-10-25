use revm::{
    context::{
        result::{InvalidHeader, InvalidTransaction},
        Block, Cfg, ContextTr, Transaction, TransactionType,
    },
    handler::validation::validate_priority_fee_tx,
};
use revm_primitives::hardfork::SpecId;

/// Validates the block and transaction environment against the configuration.
///
/// This function is based on the implementation from `revm`, but defines a custom implementation
/// of `validate_tx_env` to explicitly disable EIP-4844 transactions, which are not supported in
/// Alpen.
///
/// <https://github.com/bluealloy/revm/blob/v78/crates/handler/src/validation.rs#L11>
pub fn validate_env<CTX: ContextTr, ERROR: From<InvalidHeader> + From<InvalidTransaction>>(
    context: CTX,
) -> Result<(), ERROR> {
    let spec = context.cfg().spec().into();
    // `prevrandao` is required for the merge
    if spec.is_enabled_in(SpecId::MERGE) && context.block().prevrandao().is_none() {
        return Err(InvalidHeader::PrevrandaoNotSet.into());
    }
    // `excess_blob_gas` is required for Cancun
    if spec.is_enabled_in(SpecId::CANCUN) && context.block().blob_excess_gas_and_price().is_none() {
        return Err(InvalidHeader::ExcessBlobGasNotSet.into());
    }
    validate_tx_env::<CTX, InvalidTransaction>(context, spec).map_err(Into::into)
}

/// Validates a transaction against the block and configuration for mainnet.
///
/// This function is excerpted from `revm`, with a key difference: it explicitly
/// invalidates EIP-4844 transactions, which are not supported in Alpen.
///
/// See original: <https://github.com/bluealloy/revm/blob/v78/crates/handler/src/validation.rs#L87>
pub fn validate_tx_env<CTX: ContextTr, Error>(
    context: CTX,
    spec_id: SpecId,
) -> Result<(), InvalidTransaction> {
    // Check if the transaction's chain id is correct
    let tx_type = context.tx().tx_type();
    let tx = context.tx();

    let base_fee = if context.cfg().is_base_fee_check_disabled() {
        None
    } else {
        Some(context.block().basefee() as u128)
    };

    let tx_type = TransactionType::from(tx_type);

    // Check chain_id if config is enabled.
    // EIP-155: Simple replay attack protection
    if context.cfg().tx_chain_id_check() {
        if let Some(chain_id) = tx.chain_id() {
            if chain_id != context.cfg().chain_id() {
                return Err(InvalidTransaction::InvalidChainId);
            }
        } else if !tx_type.is_legacy() && !tx_type.is_custom() {
            // Legacy transaction are the only one that can omit chain_id.
            return Err(InvalidTransaction::MissingChainId);
        }
    }

    // EIP-7825: Transaction Gas Limit Cap
    let cap = context.cfg().tx_gas_limit_cap();
    if tx.gas_limit() > cap {
        return Err(InvalidTransaction::TxGasLimitGreaterThanCap {
            gas_limit: tx.gas_limit(),
            cap,
        });
    }

    let disable_priority_fee_check = context.cfg().is_priority_fee_check_disabled();

    match tx_type {
        TransactionType::Legacy => {
            // Gas price must be at least the basefee.
            if let Some(base_fee) = base_fee {
                if tx.gas_price() < base_fee {
                    return Err(InvalidTransaction::GasPriceLessThanBasefee);
                }
            }
        }
        TransactionType::Eip2930 => {
            // Enabled in BERLIN hardfork
            if !spec_id.is_enabled_in(SpecId::BERLIN) {
                return Err(InvalidTransaction::Eip2930NotSupported);
            }

            // Gas price must be at least the basefee.
            if let Some(base_fee) = base_fee {
                if tx.gas_price() < base_fee {
                    return Err(InvalidTransaction::GasPriceLessThanBasefee);
                }
            }
        }
        TransactionType::Eip1559 => {
            if !spec_id.is_enabled_in(SpecId::LONDON) {
                return Err(InvalidTransaction::Eip1559NotSupported);
            }
            validate_priority_fee_tx(
                tx.max_fee_per_gas(),
                tx.max_priority_fee_per_gas().unwrap_or_default(),
                base_fee,
                disable_priority_fee_check,
            )?;
        }
        // EIP-4844 transactions are not supported in alpen
        TransactionType::Eip4844 => {
            return Err(InvalidTransaction::Eip4844NotSupported);
        }
        TransactionType::Eip7702 => {
            // Check if EIP-7702 transaction is enabled.
            if !spec_id.is_enabled_in(SpecId::PRAGUE) {
                return Err(InvalidTransaction::Eip7702NotSupported);
            }

            validate_priority_fee_tx(
                tx.max_fee_per_gas(),
                tx.max_priority_fee_per_gas().unwrap_or_default(),
                base_fee,
                disable_priority_fee_check,
            )?;

            let auth_list_len = tx.authorization_list_len();
            // The transaction is considered invalid if the length of authorization_list is zero.
            if auth_list_len == 0 {
                return Err(InvalidTransaction::EmptyAuthorizationList);
            }
        }
        /* // TODO(EOF) EOF removed from spec.
        TransactionType::Eip7873 => {
            // Check if EIP-7873 transaction is enabled.
            if !spec_id.is_enabled_in(SpecId::OSAKA) {
            return Err(InvalidTransaction::Eip7873NotSupported);
            }
            // validate chain id
            if Some(context.cfg().chain_id()) != tx.chain_id() {
                return Err(InvalidTransaction::InvalidChainId);
            }

            // validate initcodes.
            validate_eip7873_initcodes(tx.initcodes())?;

            // InitcodeTransaction is invalid if the to is nil.
            if tx.kind().is_create() {
                return Err(InvalidTransaction::Eip7873MissingTarget);
            }

            validate_priority_fee_tx(
                tx.max_fee_per_gas(),
                tx.max_priority_fee_per_gas().unwrap_or_default(),
                base_fee,
            )?;
        }
        */
        TransactionType::Custom => {
            // Custom transaction type check is not done here.
        }
    };

    // Check if gas_limit is more than block_gas_limit
    if !context.cfg().is_block_gas_limit_disabled() && tx.gas_limit() > context.block().gas_limit()
    {
        return Err(InvalidTransaction::CallerGasLimitMoreThanBlock);
    }

    // EIP-3860: Limit and meter initcode. Still valid with EIP-7907 and increase of initcode size.
    if spec_id.is_enabled_in(SpecId::SHANGHAI)
        && tx.kind().is_create()
        && context.tx().input().len() > context.cfg().max_initcode_size()
    {
        return Err(InvalidTransaction::CreateInitCodeSizeLimit);
    }

    Ok(())
}
