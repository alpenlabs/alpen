//! Builders for constructing test data.

use digest::Digest;
use sha2::Sha256;
use strata_codec::{Codec, encode_to_vec};
use strata_ee_acct_types::{
    CommitBlockData, CommitChainSegment, EeHeaderBuilder, ExecBlock, ExecHeader, ExecPartialState,
    ExecutionEnvironment, PendingInputEntry,
};
use strata_ee_chain_types::{
    BlockInputs, ExecBlockCommitment, ExecBlockNotpackage, SubjectDepositData,
};

use super::errors::{BuilderError, BuilderResult};
use crate::{exec_processing::validate_block_inputs, verification_state::InputTracker};

/// Builder for constructing a chain of blocks as a segment.
///
/// This builder manages the pending inputs queue and helps construct blocks
/// that consume those inputs in order.
pub struct ChainSegmentBuilder<E: ExecutionEnvironment> {
    ee: E,
    blocks: Vec<CommitBlockData>,
    current_state: E::PartialState,
    current_header: <E::Block as ExecBlock>::Header,
    pending_inputs: Vec<PendingInputEntry>,
    consumed_inputs: usize,
}

impl<E: ExecutionEnvironment> ChainSegmentBuilder<E> {
    /// Creates a new chain segment builder from an existing state, header, and
    /// pending inputs queue.
    pub fn new(
        ee: E,
        state: E::PartialState,
        header: <E::Block as ExecBlock>::Header,
        pending_inputs: Vec<PendingInputEntry>,
    ) -> Self {
        Self {
            ee,
            blocks: Vec::new(),
            current_state: state,
            current_header: header,
            pending_inputs,
            consumed_inputs: 0,
        }
    }

    /// Returns a reference to all pending inputs (not yet consumed).
    pub fn pending_inputs(&self) -> &[PendingInputEntry] {
        &self.pending_inputs[self.consumed_inputs..]
    }

    /// Returns the next N pending inputs that could be included in a block.
    pub fn next_inputs(&self, max_count: usize) -> &[PendingInputEntry] {
        let remaining = self.pending_inputs();
        let count = max_count.min(remaining.len());
        &remaining[..count]
    }

    /// Returns the current execution state.
    pub fn current_state(&self) -> &E::PartialState {
        &self.current_state
    }

    /// Returns the current header.
    pub fn current_header(&self) -> &<E::Block as ExecBlock>::Header {
        &self.current_header
    }

    /// Returns the new execution tip block ID if there are any blocks.
    pub fn new_tip_blkid(&self) -> Option<[u8; 32]> {
        self.blocks.last().map(|b| b.notpackage().exec_blkid())
    }

    /// Appends a block body to the chain segment, consuming pending inputs.
    ///
    /// The `inputs` parameter specifies which inputs to include in this block.
    /// These inputs are validated against the internally-tracked pending inputs.
    /// The `make_header` function is called with the computed state root and
    /// should construct the appropriate header for the block.
    pub fn append_block_body<HB: EeHeaderBuilder<E>>(
        &mut self,
        header_finalizer: &HB,
        header_intrin: &HB::Intrinsics,
        body: <E::Block as ExecBlock>::Body,
        inputs: BlockInputs,
    ) -> BuilderResult<()> {
        // 1. Validate provided inputs against pending inputs using InputTracker.
        let mut tracker = InputTracker::new(self.pending_inputs());
        validate_block_inputs(&mut tracker, &inputs)?;
        let input_count = inputs.total_inputs();

        // 2. Execute the block body.
        let exec_output = self
            .ee
            .execute_block_body(&self.current_state, &body, &inputs)?;

        // 3. Create the header using the provided function.
        let header = header_finalizer.finalize_header(
            header_intrin,
            &self.current_header,
            &body,
            &exec_output,
        )?;

        // 4. Apply the write batch to compute the new state root.
        let mut post_state = self.current_state.clone();
        self.ee
            .merge_write_into_state(&mut post_state, exec_output.write_batch())?;

        // For testing, sanity check that the state root matches.
        #[cfg(any(test, debug_assertions))]
        {
            assert_eq!(
                header.get_state_root(),
                post_state
                    .compute_state_root()
                    .expect("chseg/builder: compute state root"),
                "chseg/builder: state root mismatch"
            );
        }

        // 5. Create the complete block using from_parts.
        let block = E::Block::from_parts(header.clone(), body);

        // 6. Encode the block.
        let raw_block = encode_to_vec(&block)?;

        // 7. Compute commitments.
        let exec_blkid = header.compute_block_id();
        let raw_block_hash = Sha256::digest(&raw_block).into();
        let commitment = ExecBlockCommitment::new(exec_blkid, raw_block_hash);

        // 8. Create the notpackage.
        let notpackage =
            ExecBlockNotpackage::new(commitment, inputs, exec_output.outputs().clone());

        // 9. Add to the chain
        let block_data = CommitBlockData::new(notpackage, raw_block);
        self.blocks.push(block_data);
        self.current_state = post_state;
        self.current_header = header;
        self.consumed_inputs += input_count;

        Ok(())
    }

    /// Builds the chain segment, consuming the builder.
    pub fn build(self) -> CommitChainSegment {
        CommitChainSegment::new(self.blocks)
    }

    /// Returns the number of inputs consumed so far.
    pub fn consumed_inputs(&self) -> usize {
        self.consumed_inputs
    }

    /// Returns the total number of pending inputs (consumed + remaining).
    pub fn total_inputs(&self) -> usize {
        self.pending_inputs.len()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_acct_types::{EeHeaderBuilder, ExecBlockOutput, PendingInputEntry};
    use strata_ee_chain_types::{BlockInputs, SubjectDepositData};

    use super::*;
    use crate::test_utils::{
        DummyBlock, DummyBlockBody, DummyExecutionEnvironment, DummyHeader, DummyPartialState,
        DummyTransaction,
    };

    /// Simple header builder for tests that just increments the index.
    struct TestHeaderBuilder;

    impl EeHeaderBuilder<DummyExecutionEnvironment> for TestHeaderBuilder {
        type Intrinsics = ();

        fn finalize_header(
            &self,
            _intrin: &Self::Intrinsics,
            prev_header: &DummyHeader,
            _body: &DummyBlockBody,
            exec_output: &ExecBlockOutput<DummyExecutionEnvironment>,
        ) -> strata_ee_acct_types::EnvResult<DummyHeader> {
            // Compute state root by applying the write batch
            let post_state = DummyPartialState::new(exec_output.write_batch().accounts().clone());
            let state_root = post_state.compute_state_root()?;

            Ok(DummyHeader::new(
                prev_header.compute_block_id(),
                state_root,
                prev_header.index + 1,
            ))
        }
    }

    #[test]
    fn test_chain_segment_builder_empty() {
        let ee = DummyExecutionEnvironment;
        let state = DummyPartialState::new_empty();
        let header = DummyHeader::genesis();
        let pending_inputs = vec![];

        let builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);
        let segment = builder.build();

        assert_eq!(segment.blocks().len(), 0);
    }

    #[test]
    fn test_chain_segment_builder_single_block_no_inputs() {
        let ee = DummyExecutionEnvironment;
        let state = DummyPartialState::new_empty();
        let header = DummyHeader::genesis();
        let pending_inputs = vec![];

        let mut builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);

        // Create an empty block body
        let body = DummyBlockBody::new(vec![]);
        let inputs = BlockInputs::new_empty();
        let header_builder = TestHeaderBuilder;

        builder
            .append_block_body(&header_builder, &(), body, inputs)
            .expect("append block should succeed");

        let segment = builder.build();
        assert_eq!(segment.blocks().len(), 1);
    }

    #[test]
    fn test_chain_segment_builder_with_deposit() {
        let ee = DummyExecutionEnvironment;

        // Create initial state with no accounts
        let state = DummyPartialState::new_empty();
        let header = DummyHeader::genesis();

        // Create a pending deposit
        let dest = SubjectId::from([1u8; 32]);
        let value = BitcoinAmount::from(1000u64);
        let deposit = SubjectDepositData::new(dest, value);
        let pending_inputs = vec![PendingInputEntry::Deposit(deposit.clone())];

        let mut builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);

        // Create a block that consumes the deposit
        let body = DummyBlockBody::new(vec![]);
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(deposit);
        let header_builder = TestHeaderBuilder;

        builder
            .append_block_body(&header_builder, &(), body, inputs)
            .expect("append block should succeed");

        assert_eq!(builder.consumed_inputs(), 1);
        assert_eq!(builder.pending_inputs().len(), 0);

        let segment = builder.build();
        assert_eq!(segment.blocks().len(), 1);

        // Verify the block consumed the deposit
        let block_data = &segment.blocks()[0];
        assert_eq!(block_data.notpackage().inputs().subject_deposits().len(), 1);
    }

    #[test]
    fn test_chain_segment_builder_multiple_blocks() {
        let ee = DummyExecutionEnvironment;

        // Create initial state
        let mut initial_accounts = std::collections::BTreeMap::new();
        let alice = SubjectId::from([1u8; 32]);
        initial_accounts.insert(alice, 1000u64);
        let state = DummyPartialState::new(initial_accounts);
        let header = DummyHeader::genesis();

        // Create deposits for two blocks
        let bob = SubjectId::from([2u8; 32]);
        let deposit1 = SubjectDepositData::new(bob, BitcoinAmount::from(500u64));
        let deposit2 = SubjectDepositData::new(bob, BitcoinAmount::from(300u64));
        let pending_inputs = vec![
            PendingInputEntry::Deposit(deposit1.clone()),
            PendingInputEntry::Deposit(deposit2.clone()),
        ];

        let mut builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);
        let header_builder = TestHeaderBuilder;

        // First block: consume first deposit
        let body1 = DummyBlockBody::new(vec![]);
        let mut inputs1 = BlockInputs::new_empty();
        inputs1.add_subject_deposit(deposit1);

        builder
            .append_block_body(&header_builder, &(), body1, inputs1)
            .expect("append first block should succeed");

        assert_eq!(builder.consumed_inputs(), 1);
        assert_eq!(builder.pending_inputs().len(), 1);

        // Second block: consume second deposit and do a transfer
        let transfer = DummyTransaction::Transfer {
            from: alice,
            to: bob,
            value: 100,
        };
        let body2 = DummyBlockBody::new(vec![transfer]);
        let mut inputs2 = BlockInputs::new_empty();
        inputs2.add_subject_deposit(deposit2);

        builder
            .append_block_body(&header_builder, &(), body2, inputs2)
            .expect("append second block should succeed");

        assert_eq!(builder.consumed_inputs(), 2);
        assert_eq!(builder.pending_inputs().len(), 0);

        let segment = builder.build();
        assert_eq!(segment.blocks().len(), 2);
    }

    #[test]
    fn test_chain_segment_builder_input_mismatch() {
        let ee = DummyExecutionEnvironment;
        let state = DummyPartialState::new_empty();
        let header = DummyHeader::genesis();

        let dest1 = SubjectId::from([1u8; 32]);
        let deposit1 = SubjectDepositData::new(dest1, BitcoinAmount::from(1000u64));
        let pending_inputs = vec![PendingInputEntry::Deposit(deposit1)];

        let mut builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);

        // Try to append a block with a different deposit
        let dest2 = SubjectId::from([2u8; 32]);
        let deposit2 = SubjectDepositData::new(dest2, BitcoinAmount::from(1000u64));
        let body = DummyBlockBody::new(vec![]);
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(deposit2);
        let header_builder = TestHeaderBuilder;

        let result = builder.append_block_body(&header_builder, &(), body, inputs);
        assert!(result.is_err());
    }

    #[test]
    fn test_chain_segment_builder_insufficient_inputs() {
        let ee = DummyExecutionEnvironment;
        let state = DummyPartialState::new_empty();
        let header = DummyHeader::genesis();

        let dest = SubjectId::from([1u8; 32]);
        let deposit = SubjectDepositData::new(dest, BitcoinAmount::from(1000u64));
        let pending_inputs = vec![PendingInputEntry::Deposit(deposit.clone())];

        let mut builder = ChainSegmentBuilder::new(ee, state, header, pending_inputs);

        // Try to append a block that wants more inputs than available
        let body = DummyBlockBody::new(vec![]);
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(deposit.clone());
        inputs.add_subject_deposit(deposit); // Add a second one that doesn't exist
        let header_builder = TestHeaderBuilder;

        let result = builder.append_block_body(&header_builder, &(), body, inputs);
        assert!(result.is_err());
    }
}
