# ETH-EE Account Proof Implementation

This crate provides **only the guest-side** proof generation logic for ETH-EE account updates.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ HOST APPLICATION (NOT in this crate)                         │
├─────────────────────────────────────────────────────────────┤
│  • Implement EthEeAcctDataProvider trait                     │
│  • Implement prepare_proof_input()                           │
│  • Fetch data from database/storage                          │
└─────────────────────────────────────────────────────────────┘
                         ↓ (EthEeAcctInput)
┌─────────────────────────────────────────────────────────────┐
│ THIS CRATE (Guest code only)                                 │
├─────────────────────────────────────────────────────────────┤
│  • process_eth_ee_acct_update() - verification logic         │
│  • EthEeAcctProgram - zkVM program definition                │
│  • EthEeAcctInput/Output - I/O types                         │
└─────────────────────────────────────────────────────────────┘
```

## Host Implementation Required (Tentative)

The host application should implement:

### Data Provider Trait
```rust
pub trait EthEeAcctDataProvider: Send + Sync {
    fn fetch_ee_account_state(&self, account_id: AccountId) -> Result<EeAccountState>;
    fn fetch_update_operation(&self, update_id: UpdateId) -> Result<UpdateOperationData>;
    fn fetch_chain_segments(&self, update_id: UpdateId) -> Result<Vec<CommitChainSegment>>;
    fn fetch_previous_header(&self, exec_blkid: Hash) -> Result<Vec<u8>>;
    fn fetch_partial_state(&self, exec_blkid: Hash) -> Result<Vec<u8>>;
}
```

### Input Preparation Function
```rust
pub fn prepare_proof_input(
    provider: &impl EthEeAcctDataProvider,
    account_id: AccountId,
    update_id: UpdateId,
    genesis: rsp_primitives::genesis::Genesis,
) -> Result<EthEeAcctInput>
```
