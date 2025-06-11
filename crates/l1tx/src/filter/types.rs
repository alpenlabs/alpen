use bitcoin::{hashes::Hash, Amount};
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{
    block_credential::CredRule,
    buf::Buf32,
    l1::{BitcoinAddress, BitcoinAmount, BitcoinScriptBuf, OutputRef, XOnlyPk},
    params::{DepositTxParams, RollupParams},
    sorted_vec::{FlatTable, SortedVec, TableEntry},
};
use strata_state::{
    bridge_state::{DepositEntry, DepositState},
    chain_state::Chainstate,
};

use crate::utils::{generate_taproot_address, get_operator_wallet_pks};

// TODO: This is FIXED OPERATOR FEE for TN1
pub const OPERATOR_FEE: Amount = Amount::from_int_btc(2);

/// A configuration that determines how relevant transactions in a bitcoin block are filtered.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct TxFilterConfig {
    /// Envelope tag names
    pub envelope_tags: EnvelopeTags,

    /// Rules for verifying sequencer signature
    pub sequencer_cred_rule: CredRule,

    /// For addresses that are expected to be spent to.
    pub expected_addrs: SortedVec<BitcoinAddress>,

    /// For blobs that are expected to be written to bitcoin.
    ///
    /// This might be removed soon.
    pub expected_blobs: SortedVec<Buf32>,

    /// For deposits that might be spent from.
    pub expected_outpoints: FlatTable<DepositUtxoInfo>,

    /// For withdrawal fulfillment transactions sent by bridge operators.
    ///
    /// Maps deposit idx to fulfillment data.
    pub expected_withdrawal_fulfillments: FlatTable<WithdrawalCommandInfo>,

    /// Deposit config that determines how a deposit transaction can be parsed.
    pub deposit_config: DepositTxParams,
}

impl TxFilterConfig {
    /// Derive a `TxFilterConfig` from `RollupParams`.
    // TODO: this will need chainstate too in the future
    pub fn derive_from(rollup_params: &RollupParams) -> anyhow::Result<Self> {
        let operator_wallet_pks = get_operator_wallet_pks(rollup_params);
        let (address, int_pubkey) =
            generate_taproot_address(&operator_wallet_pks, rollup_params.network)?;

        let rollup_name = rollup_params.rollup_name.clone();

        // Make sure the magic bytes is always of length 4.
        // TODO replace this with a more generic system
        let magic_bytes = rollup_name
            .bytes()
            .chain(std::iter::repeat(0))
            .take(4)
            .collect::<Vec<_>>();

        let expected_addrs = SortedVec::new_unchecked(vec![address.clone()]);
        let sequencer_cred_rule = rollup_params.cred_rule.clone();

        let envelope_tags = EnvelopeTags {
            checkpoint_tag: rollup_params.checkpoint_tag.clone(),
            da_tag: rollup_params.da_tag.clone(),
        };

        let operators_pubkey =
            XOnlyPk::new(int_pubkey.serialize().into()).expect("Aggregated pubkey should be valid");

        let deposit_config = DepositTxParams {
            magic_bytes,
            address_length: rollup_params.address_length,
            deposit_amount: rollup_params.deposit_amount,
            address,
            operators_pubkey,
        };

        Ok(Self {
            envelope_tags,
            sequencer_cred_rule,
            expected_addrs,
            expected_blobs: SortedVec::new_empty(),
            expected_outpoints: FlatTable::new_empty(),
            expected_withdrawal_fulfillments: FlatTable::new_empty(),
            deposit_config,
        })
    }

    pub fn update_from_chainstate(&mut self, chainstate: &Chainstate) {
        // Watch all withdrawals that have been ordered.
        let exp_fulfillments = chainstate
            .deposits_table()
            .deposits()
            .flat_map(conv_deposit_to_fulfillment)
            .collect::<Vec<_>>();

        self.expected_withdrawal_fulfillments = FlatTable::try_from_unsorted(exp_fulfillments)
            .expect(
                "types: duplicate/unsorted deposit indexes? (expected_withdrawal_fulfillments)?",
            );

        // Watch all utxos we have in our deposit table.
        let exp_outpoints = chainstate
            .deposits_table()
            .deposits()
            .map(|deposit| DepositUtxoInfo {
                deposit_idx: deposit.idx(),
                output: *deposit.output(),
            })
            .collect::<Vec<_>>();

        self.expected_outpoints = FlatTable::try_from_unsorted(exp_outpoints)
            .expect("types: duplicate/unsorted deposit indexes? (expected_outpoints)");
    }
}

/// The tags used for the two envelope kinds we recognize.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EnvelopeTags {
    pub checkpoint_tag: String,
    pub da_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct DepositUtxoInfo {
    /// The utxo's outpoint.
    ///
    /// This is used as the key in the expected outpoints table.
    pub output: OutputRef,

    /// Deposit index that this utxo corresponds to.
    pub deposit_idx: u32,
}

impl TableEntry for DepositUtxoInfo {
    type Key = OutputRef;
    fn get_key(&self) -> &Self::Key {
        &self.output
    }
}

/// Describes information we expect to see about a withdrawal fulfillment.
///
/// This is extracted directly from the deposit state, if it's in the dispatched
/// state.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct WithdrawalCommandInfo {
    /// Index of the deposit in the deposits table.  This is also the key in the
    /// expected withdrawals table that we perform filtering with.
    pub deposit_idx: u32,

    /// The operator ordered to fulfill the withdrawal.
    pub operator_idx: u32,

    // TODO make this a vec of outputs along with amt
    /// Expected destination script buf.
    pub destination: BitcoinScriptBuf,

    /// Expected minimum withdrawal amount in sats.
    pub min_amount: BitcoinAmount,

    /// Txid of the locked deposit utxo, which will ultimately be claimed by
    /// the operator.
    pub deposit_txid: [u8; 32],
}

impl TableEntry for WithdrawalCommandInfo {
    type Key = u32;
    fn get_key(&self) -> &Self::Key {
        &self.deposit_idx
    }
}

pub fn conv_deposit_to_fulfillment(entry: &DepositEntry) -> Option<WithdrawalCommandInfo> {
    let DepositState::Dispatched(state) = entry.deposit_state() else {
        return None;
    };

    // Sanity check until we actually support multiple outputs.
    let noutputs = state.cmd().withdraw_outputs().len();
    if noutputs != 1 {
        panic!("l1txfilter: withdrawal dispatch with {noutputs} (exp 1)");
    }

    let outp = &state.cmd().withdraw_outputs()[0];

    // TODO move this fee calculation somewhere else more intelligent
    let amount = outp.amt().to_sat().saturating_sub(OPERATOR_FEE.to_sat());
    let deposit_txid = entry.output().outpoint().txid.as_raw_hash().to_byte_array();

    Some(WithdrawalCommandInfo {
        deposit_idx: entry.idx(),
        operator_idx: state.assignee(),
        destination: outp.destination().to_script().into(),
        min_amount: BitcoinAmount::from_sat(amount),
        deposit_txid,
    })
}
