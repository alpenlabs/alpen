use std::sync::Arc;

use alloy_consensus::Transaction;
use alloy_eips::Typed2718;
use alpen_reth_evm::collect_withdrawal_intents;
use alpen_reth_primitives::WithdrawalIntent;
use reth::{
    builder::BuilderContext, providers::StateProviderFactory, revm::database::StateProviderDatabase,
};
use reth_basic_payload_builder::*;
use reth_chainspec::{ChainSpec, ChainSpecProvider, EthChainSpec, EthereumHardforks};
use reth_errors::{BlockExecutionError, BlockValidationError};
use reth_ethereum_payload_builder::EthereumBuilderConfig;
use reth_ethereum_primitives::TransactionSigned;
use reth_evm::{
    execute::{BlockBuilder, BlockBuilderOutcome},
    Evm, NextBlockEnvAttributes,
};
use reth_evm_ethereum::EthEvmConfig;
use reth_node_api::{ConfigureEvm, FullNodeTypes, NodeTypes, PayloadBuilderAttributes};
use reth_node_builder::{components::PayloadBuilderBuilder, PayloadBuilderConfig};
use reth_payload_builder::{EthBuiltPayload, PayloadBuilderError};
use reth_primitives::{EthPrimitives, InvalidTransactionError, Receipt};
use reth_primitives_traits::SignedTransaction;
use reth_transaction_pool::{
    error::{Eip4844PoolTransactionError, InvalidPoolTransactionError},
    BestTransactions, BestTransactionsAttributes, PoolTransaction, TransactionPool,
    ValidPoolTransaction,
};
use revm::{context::Block, database::State};
use revm_primitives::U256;
use tracing::{debug, trace, warn};

use crate::{
    engine::StrataEngineTypes,
    evm::StrataEvmFactory,
    payload::{StrataBuiltPayload, StrataPayloadBuilderAttributes},
};

/// A custom payload service builder that supports the custom engine types
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct StrataPayloadBuilderBuilder;

impl<Node, Pool> PayloadBuilderBuilder<Node, Pool> for StrataPayloadBuilderBuilder
where
    Node: FullNodeTypes<
        Types: NodeTypes<
            Payload = StrataEngineTypes,
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
        >,
    >,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>
        + Unpin
        + 'static,
{
    type PayloadBuilder = StrataPayloadBuilder<Pool, Node::Provider>;

    async fn build_payload_builder(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
    ) -> eyre::Result<Self::PayloadBuilder> {
        let conf = ctx.payload_builder_config();
        Ok(StrataPayloadBuilder::new(
            ctx.provider().clone(),
            pool,
            ctx.chain_spec().clone(),
            EthereumBuilderConfig::new().with_gas_limit(conf.gas_limit()),
        ))
    }
}

/// The type responsible for building custom payloads
#[derive(Debug, Clone)]
#[non_exhaustive]
<<<<<<< HEAD
pub(crate) struct StrataPayloadBuilder {
=======
pub struct StrataPayloadBuilder<Pool, Client> {
    /// Client providing access to node state.
    client: Client,
    /// Transaction pool.
    pool: Pool,
>>>>>>> 3b67491a (bump reth to v1.3.1)
    /// The type responsible for creating the evm.
    evm_config: EthEvmConfig<StrataEvmFactory>,
    /// Payload builder configuration.
    builder_config: EthereumBuilderConfig,
}

<<<<<<< HEAD
impl StrataPayloadBuilder {
    /// Returns the configured [`CfgEnvWithHandlerCfg`] and [`BlockEnv`] for the targeted payload
    /// (that has the `parent` as its parent).
    pub(crate) fn cfg_and_block_env(
        &self,
        attributes: &StrataPayloadBuilderAttributes,
        parent: &Header,
    ) -> Result<EvmEnv, <StrataEvmConfig as ConfigureEvmEnv>::Error> {
        let next_attributes = NextBlockEnvAttributes {
            timestamp: attributes.timestamp(),
            suggested_fee_recipient: attributes.suggested_fee_recipient(),
            prev_randao: attributes.prev_randao(),
            gas_limit: parent.gas_limit,
        };
        self.evm_config
            .next_cfg_and_block_env(parent, next_attributes)
=======
impl<Pool, Client> StrataPayloadBuilder<Pool, Client> {
    /// `StrataPayloadBuilder` constructor.
    pub fn new(
        client: Client,
        pool: Pool,
        chain_spec: Arc<ChainSpec>,
        builder_config: EthereumBuilderConfig,
    ) -> Self {
        Self {
            client,
            pool,
            evm_config: EthEvmConfig::new_with_evm_factory(chain_spec, StrataEvmFactory::default()),
            builder_config,
        }
>>>>>>> 3b67491a (bump reth to v1.3.1)
    }
}

impl<Pool, Client> PayloadBuilder for StrataPayloadBuilder<Pool, Client>
where
    Client: StateProviderFactory + ChainSpecProvider<ChainSpec = ChainSpec> + Clone,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>,
{
    type Attributes = StrataPayloadBuilderAttributes;
    type BuiltPayload = StrataBuiltPayload;

    fn try_build(
        &self,
        args: BuildArguments<Self::Attributes, Self::BuiltPayload>,
    ) -> Result<BuildOutcome<Self::BuiltPayload>, PayloadBuilderError> {
        try_build_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            self.builder_config.clone(),
            args,
            |attributes| self.pool.best_transactions_with_attributes(attributes),
        )
    }

    fn build_empty_payload(
        &self,
        config: PayloadConfig<Self::Attributes>,
    ) -> Result<Self::BuiltPayload, PayloadBuilderError> {
        let args = BuildArguments::new(Default::default(), config, Default::default(), None);
        try_build_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            self.builder_config.clone(),
            args,
            |attributes| self.pool.best_transactions_with_attributes(attributes),
        )?
        .into_payload()
        .ok_or_else(|| PayloadBuilderError::MissingPayload)
    }
}

type BestTransactionsIter<Pool> = Box<
    dyn BestTransactions<Item = Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>,
>;

/// Constructs an Ethereum transaction payload using the best transactions from the pool.
///
/// Given build arguments including an Ethereum client, transaction pool,
/// and configuration, this function creates a transaction payload. Returns
/// a res ult indicating success with the payload or an error in case of failure.
///
/// Adapted from
/// [default_ethereum_payload](reth_ethereum_payload_builder::default_ethereum_payload)
#[inline]
pub fn try_build_payload<EvmConfig, Pool, Client, F>(
    evm_config: EvmConfig,
    client: Client,
    pool: Pool,
    builder_config: EthereumBuilderConfig,
    args: BuildArguments<StrataPayloadBuilderAttributes, StrataBuiltPayload>,
    best_txs: F,
) -> Result<BuildOutcome<StrataBuiltPayload>, PayloadBuilderError>
where
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
    Client: StateProviderFactory + ChainSpecProvider<ChainSpec = ChainSpec>,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>,
    F: FnOnce(BestTransactionsAttributes) -> BestTransactionsIter<Pool>,
{
    let BuildArguments {
        mut cached_reads,
        config,
        cancel,
        best_payload,
    } = args;
    let PayloadConfig {
        parent_header,
        attributes,
    } = config;

    let batch_gas_limit = attributes.batch_gas_limit();
    let attributes = attributes.inner;

    let state_provider = client.state_by_block_hash(parent_header.hash())?;
    let state = StateProviderDatabase::new(&state_provider);
    let mut db = State::builder()
        .with_database(cached_reads.as_db_mut(state))
        .with_bundle_update()
        .build();

    let mut builder = evm_config
        .builder_for_next_block(
            &mut db,
            &parent_header,
            NextBlockEnvAttributes {
                timestamp: attributes.timestamp(),
                suggested_fee_recipient: attributes.suggested_fee_recipient(),
                prev_randao: attributes.prev_randao(),
                gas_limit: builder_config.gas_limit(parent_header.gas_limit),
                parent_beacon_block_root: attributes.parent_beacon_block_root(),
                withdrawals: Some(attributes.withdrawals().clone()),
            },
        )
        .map_err(PayloadBuilderError::other)?;

    let chain_spec = client.chain_spec();

    debug!(target: "payload_builder", id=%attributes.id, parent_header = ?parent_header.hash(), parent_number = parent_header.number, "building new payload");
    let mut cumulative_gas_used = 0;
    let env_block_gas_limit: u64 = builder.evm_mut().block().gas_limit;
    let block_gas_limit = batch_gas_limit
        .map(|batch_gas_limit| batch_gas_limit.min(env_block_gas_limit))
        .unwrap_or(env_block_gas_limit);

    let base_fee = builder.evm_mut().block().basefee;

    let mut best_txs = best_txs(BestTransactionsAttributes::new(
        base_fee,
        builder
            .evm_mut()
            .block()
            .blob_gasprice()
            .map(|gasprice| gasprice as u64),
    ));
    let mut total_fees = U256::ZERO;

    builder.apply_pre_execution_changes().map_err(|err| {
        warn!(target: "payload_builder", %err, "failed to apply pre-execution changes");
        PayloadBuilderError::Internal(err.into())
    })?;

    let mut block_blob_count = 0;
    let blob_params = chain_spec.blob_params_at_timestamp(attributes.timestamp);
    let max_blob_count = blob_params
        .as_ref()
        .map(|params| params.max_blob_count)
        .unwrap_or_default();

    while let Some(pool_tx) = best_txs.next() {
        // ensure we still have capacity for this transaction
        if cumulative_gas_used + pool_tx.gas_limit() > block_gas_limit {
            // we can't fit this transaction into the block, so we need to mark it as invalid
            // which also removes all dependent transaction from the iterator before we can
            // continue
            best_txs.mark_invalid(
                &pool_tx,
                InvalidPoolTransactionError::ExceedsGasLimit(pool_tx.gas_limit(), block_gas_limit),
            );
            continue;
        }

        // check if the job was cancelled, if so we can exit early
        if cancel.is_cancelled() {
            return Ok(BuildOutcome::Cancelled);
        }

        // convert tx to a signed transaction
        let tx = pool_tx.to_consensus();

        // There's only limited amount of blob space available per block, so we need to check if
        // the EIP-4844 can still fit in the block
        if let Some(blob_tx) = tx.as_eip4844() {
            let tx_blob_count = blob_tx.blob_versioned_hashes.len() as u64;

            if block_blob_count + tx_blob_count > max_blob_count {
                // we can't fit this _blob_ transaction into the block, so we mark it as
                // invalid, which removes its dependent transactions from
                // the iterator. This is similar to the gas limit condition
                // for regular transactions above.
                trace!(target: "payload_builder", tx=?tx.hash(), ?block_blob_count, "skipping blob transaction because it would exceed the max blob count per block");
                best_txs.mark_invalid(
                    &pool_tx,
                    InvalidPoolTransactionError::Eip4844(
                        Eip4844PoolTransactionError::TooManyEip4844Blobs {
                            have: block_blob_count + tx_blob_count,
                            permitted: max_blob_count,
                        },
                    ),
                );
                continue;
            }
        }

        let gas_used = match builder.execute_transaction(tx.clone()) {
            Ok(gas_used) => gas_used,
            Err(BlockExecutionError::Validation(BlockValidationError::InvalidTx {
                error, ..
            })) => {
                if error.is_nonce_too_low() {
                    // if the nonce is too low, we can skip this transaction
                    trace!(target: "payload_builder", %error, ?tx, "skipping nonce too low transaction");
                } else {
                    // if the transaction is invalid, we can skip it and all of its
                    // descendants
                    trace!(target: "payload_builder", %error, ?tx, "skipping invalid transaction and its descendants");
                    best_txs.mark_invalid(
                        &pool_tx,
                        InvalidPoolTransactionError::Consensus(
                            InvalidTransactionError::TxTypeNotSupported,
                        ),
                    );
                }
                continue;
            }
            // this is an error that we should treat as fatal for this attempt
            Err(err) => return Err(PayloadBuilderError::evm(err)),
        };

        // add to the total blob gas used if the transaction successfully executed
        if let Some(blob_tx) = tx.as_eip4844() {
            block_blob_count += blob_tx.blob_versioned_hashes.len() as u64;

            // if we've reached the max blob count, we can skip blob txs entirely
            if block_blob_count == max_blob_count {
                best_txs.skip_blobs();
            }
        }

        // update add to total fees
        let miner_fee = tx
            .effective_tip_per_gas(base_fee)
            .expect("fee is always valid; execution succeeded");

        total_fees += U256::from(miner_fee) * U256::from(gas_used);
        cumulative_gas_used += gas_used;
    }

    // check if we have a better block
    if !is_better_payload(best_payload.as_ref(), total_fees) {
        // Release db
        drop(builder);
        // can skip building the block
        return Ok(BuildOutcome::Aborted {
            fees: total_fees,
            cached_reads,
        });
    }

    let BlockBuilderOutcome {
        execution_result,
        block,
        ..
    } = builder.finish(&state_provider)?;

    let requests = chain_spec
        .is_prague_active_at_timestamp(attributes.timestamp)
        .then_some(execution_result.requests);

    // initialize empty blob sidecars at first. If cancun is active then this will
    let mut blob_sidecars = Vec::new();

    // only determine cancun fields when active
    if chain_spec.is_cancun_active_at_timestamp(attributes.timestamp) {
        // grab the blob sidecars from the executed txs
        blob_sidecars = pool
            .get_all_blobs_exact(
                block
                    .body()
                    .transactions()
                    .filter(|tx| tx.is_eip4844())
                    .map(|tx| *tx.tx_hash())
                    .collect(),
            )
            .map_err(PayloadBuilderError::other)?;
    }

    // collect receipts from the executed transactions
    let receipts: Vec<Receipt> = execution_result.receipts;
    let txns: Vec<TransactionSigned> = block.body().transactions().cloned().collect();
    let tx_receipt_pairs = txns.iter().zip(receipts.iter());
    let withdrawal_intents: Vec<WithdrawalIntent> =
        collect_withdrawal_intents(tx_receipt_pairs).collect();

    let sealed_block = Arc::new(block.sealed_block().clone());
    debug!(target: "payload_builder", id=%attributes.id, sealed_block_header = ?sealed_block.sealed_header(), "sealed built block");

    let mut eth_payload = EthBuiltPayload::new(attributes.id, sealed_block, total_fees, requests);
    // extend the payload with the blob sidecars from the executed txs
    eth_payload.extend_sidecars(blob_sidecars.into_iter().map(Arc::unwrap_or_clone));

    let strata_payload = StrataBuiltPayload::new(eth_payload, withdrawal_intents);
    // let strata_payload = StrataBuiltPayload::new(eth_payload, Default::default());

    Ok(BuildOutcome::Better {
        payload: strata_payload,
        cached_reads,
    })
}
