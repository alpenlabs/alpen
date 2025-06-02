use std::sync::Arc;

use alpen_reth_evm::set_evm_handles;
use reth_chainspec::ChainSpec;
use reth_evm::{env::EvmEnv, ConfigureEvm, ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_node_ethereum::EthEvmConfig;
use reth_primitives::{Header, TransactionSigned};
use revm::{inspector_handle_register, Database, Evm, EvmBuilder, GetInspector};
use revm_primitives::{Address, Bytes, CfgEnvWithHandlerCfg, Env, TxEnv};

/// Custom EVM configuration
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StrataEvmConfig {
    inner: EthEvmConfig,
}

impl StrataEvmConfig {
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            inner: EthEvmConfig::new(chain_spec),
        }
    }

    pub const fn inner(&self) -> &EthEvmConfig {
        &self.inner
    }
}

impl ConfigureEvmEnv for StrataEvmConfig {
    type Header = Header;
    type Transaction = TransactionSigned;
    type Error = core::convert::Infallible;

    fn fill_cfg_env(&self, cfg_env: &mut CfgEnvWithHandlerCfg, header: &Self::Header) {
        self.inner.fill_cfg_env(cfg_env, header);
    }

    fn fill_tx_env(&self, tx_env: &mut TxEnv, transaction: &TransactionSigned, sender: Address) {
        self.inner.fill_tx_env(tx_env, transaction, sender);
    }

    fn fill_tx_env_system_contract_call(
        &self,
        env: &mut Env,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) {
        self.inner
            .fill_tx_env_system_contract_call(env, caller, contract, data);
    }

    fn next_cfg_and_block_env(
        &self,
        parent: &Self::Header,
        attributes: NextBlockEnvAttributes,
    ) -> Result<EvmEnv, Self::Error> {
        self.inner.next_cfg_and_block_env(parent, attributes)
    }
}

impl ConfigureEvm for StrataEvmConfig {
    type DefaultExternalContext<'a> = ();

    fn evm<DB: Database>(&self, db: DB) -> Evm<'_, Self::DefaultExternalContext<'_>, DB> {
        EvmBuilder::default()
            .with_db(db)
            // add additional precompiles
            .append_handler_register(set_evm_handles)
            .build()
    }

    fn evm_with_inspector<DB, I>(&self, db: DB, inspector: I) -> Evm<'_, I, DB>
    where
        DB: Database,
        I: GetInspector<DB>,
    {
        EvmBuilder::default()
            .with_db(db)
            .with_external_context(inspector)
            // add additional precompiles
            .append_handler_register(set_evm_handles)
            .append_handler_register(inspector_handle_register)
            .build()
    }

    #[doc = " Provides the default external context."]
    fn default_external_context<'a>(&self) -> Self::DefaultExternalContext<'a> {
        self.inner.default_external_context()
    }
}
