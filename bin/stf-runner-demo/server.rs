use std::net::SocketAddr;
use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error, debug};

use stf_runner::{
    account::{AccountId, AccountState},
    block::OLBlock,
};

use crate::state::{SharedDemoState, StateInfo};
use crate::generator::{
    generate_test_block, generate_invalid_block, generate_deposit_block, 
    generate_demo_transactions, parse_invalid_type, InvalidType
};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockSubmissionResult {
    pub success: bool,
    pub message: String,
    pub new_state_root: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeneratedBlock {
    pub block: OLBlock,
    pub description: String,
}

#[rpc(server)]
pub trait StfDemoRpc {
    /// Submit a block for processing
    #[method(name = "submit_block")]
    async fn submit_block(&self, block: OLBlock) -> RpcResult<BlockSubmissionResult>;
    
    /// Get current chain state
    #[method(name = "get_state")]
    async fn get_state(&self) -> RpcResult<StateInfo>;
    
    /// Get specific account state
    #[method(name = "get_account")]
    async fn get_account(&self, account_id: AccountId) -> RpcResult<Option<AccountState>>;
    
    /// Generate a test block with transactions
    #[method(name = "generate_test_block")]
    async fn generate_test_block(&self, with_txs: bool) -> RpcResult<GeneratedBlock>;
    
    /// Generate an invalid block for testing validation
    #[method(name = "generate_invalid_block")]
    async fn generate_invalid_block(&self, error_type: String) -> RpcResult<GeneratedBlock>;
    
    /// Generate a block with L1 deposits
    #[method(name = "generate_deposit_block")]
    async fn generate_deposit_block(&self, deposits: Vec<(u32, u64)>) -> RpcResult<GeneratedBlock>;
    
    /// Get demo account IDs for easy testing
    #[method(name = "get_demo_accounts")]
    async fn get_demo_accounts(&self) -> RpcResult<Vec<AccountId>>;
}

pub struct StfDemoServer {
    state: SharedDemoState,
}

impl StfDemoServer {
    pub fn new(state: SharedDemoState) -> Self {
        Self { state }
    }

    pub async fn start(self, addr: &str) -> anyhow::Result<ServerHandle> {
        let server = ServerBuilder::default()
            .build(addr.parse::<SocketAddr>()?)
            .await?;
        
        let handle = server.start(self.into_rpc())?;
        println!("STF Runner Demo server started at: http://{}", addr);
        println!("\nAvailable methods:");
        println!("  - submit_block");
        println!("  - get_state");  
        println!("  - get_account");
        println!("  - generate_test_block");
        println!("  - generate_invalid_block");
        println!("  - generate_deposit_block");
        println!("  - get_demo_accounts");
        
        Ok(handle)
    }
}

#[jsonrpsee::core::async_trait]
impl StfDemoRpcServer for StfDemoServer {
    async fn submit_block(&self, block: OLBlock) -> RpcResult<BlockSubmissionResult> {
        info!("Processing block submission...");
        let mut state = self.state.lock().unwrap();
        
        match state.process_block(block) {
            Ok(message) => {
                let state_info = state.get_state_info();
                Ok(BlockSubmissionResult {
                    success: true,
                    message,
                    new_state_root: Some(format!("{}", state_info.accounts_root)),
                })
            }
            Err(e) => {
                Ok(BlockSubmissionResult {
                    success: false,
                    message: format!("Block processing failed: {}", e),
                    new_state_root: None,
                })
            }
        }
    }

    async fn get_state(&self) -> RpcResult<StateInfo> {
        let state = self.state.lock().unwrap();
        Ok(state.get_state_info())
    }

    async fn get_account(&self, account_id: AccountId) -> RpcResult<Option<AccountState>> {
        let state = self.state.lock().unwrap();
        Ok(state.get_account(&account_id))
    }

    async fn generate_test_block(&self, with_txs: bool) -> RpcResult<GeneratedBlock> {
        let state = self.state.lock().unwrap();
        let prev_header = state.get_latest_header();
        
        let block = if with_txs {
            let mut test_block = generate_test_block(&prev_header, false);
            // Add demo transactions
            let txs = generate_demo_transactions();
            let body = test_block.body().clone();
            let new_body = stf_runner::block::OLBlockBody::new(
                body.logs().to_vec(),
                Some(txs),
                body.l1update().clone()
            );
            stf_runner::block::OLBlock::new(test_block.signed_header().clone(), new_body)
        } else {
            generate_test_block(&prev_header, false)
        };

        let description = if with_txs {
            "Valid block with demo transactions (transfers and account updates)".to_string()
        } else {
            "Valid empty block".to_string()
        };

        Ok(GeneratedBlock { block, description })
    }

    async fn generate_invalid_block(&self, error_type: String) -> RpcResult<GeneratedBlock> {
        let state = self.state.lock().unwrap();
        let prev_header = state.get_latest_header();
        
        let invalid_type = parse_invalid_type(&error_type)
            .ok_or_else(|| "Invalid error type. Use: bad_slot, bad_timestamp, bad_parent, zero_body_root")?;
        
        let block = generate_invalid_block(&prev_header, invalid_type);
        let description = match error_type.as_str() {
            "bad_slot" => "Invalid block with slot regression",
            "bad_timestamp" => "Invalid block with timestamp too far in past", 
            "bad_parent" => "Invalid block with wrong parent block ID",
            "zero_body_root" => "Invalid block with zero body root",
            _ => "Invalid block",
        }.to_string();

        Ok(GeneratedBlock { block, description })
    }

    async fn generate_deposit_block(&self, deposits: Vec<(u32, u64)>) -> RpcResult<GeneratedBlock> {
        let state = self.state.lock().unwrap();
        let prev_header = state.get_latest_header();
        
        let block = generate_deposit_block(&prev_header, deposits.clone());
        let description = format!(
            "Block with {} L1 deposit(s): {:?}",
            deposits.len(),
            deposits
        );

        Ok(GeneratedBlock { block, description })
    }

    async fn get_demo_accounts(&self) -> RpcResult<Vec<AccountId>> {
        // Return the demo account IDs that are created during initialization
        use strata_primitives::buf::Buf32;
        let accounts = vec![
            Buf32::from([0u8; 32]),
            Buf32::from([1u8; 32]), 
            Buf32::from([2u8; 32]),
        ];
        Ok(accounts)
    }
}