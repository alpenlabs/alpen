//! Golden test for V1 execution.
//!
//! `v0.4.0-rc.1` fixed the V1 checkpoint guest, and nodes keep running V1
//! natively next to the proofs it verifies, so V1 execution must not change.
//! This test builds V1 chains through the dispatch entry points and checks
//! them against the values `v0.4.0-rc.1` produces for the same inputs: every
//! block's state root and block ID, and every epoch's checkpoint claim, which
//! commits to the epoch's DA diff, logs and ASM manifests. The expected values
//! were recorded by running this file at that tag.
//!
//! The chains are built with the `strata-ol-stf-v1` test helpers, so a change
//! to those helpers also changes the values. On a mismatch, find out whether
//! V1 execution or the inputs changed before recording new values.

use std::iter;

use ssz::Encode;
use strata_acct_types::{BitcoinAmount, MessageEntry};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_identifiers::{AccountSerial, BRIDGE_GATEWAY_ACCT_ID, SubjectId};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecVersions};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf::{
    BlockComponents, BlockContext, BlockInfo, EpochInfo, OLSpecId, apply_da_epoch, construct_block,
    verify_block,
};
use strata_ol_stf_v1::test_utils::*;
use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};
use strata_predicate::PredicateKey;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};

use crate::compute_epoch_da;

/// One block of a golden chain: its slot, state root and block ID, in hex.
type GoldenBlock = (u64, &'static str, &'static str);

/// The blocks `v0.4.0-rc.1` produces for [`build_v1_genesis_chain`].
const V1_GENESIS_BLOCKS: &[GoldenBlock] = &[
    (
        0,
        "6eb06689587fd47d39bd45104813ab5486308a8b51942985887cb4a11c917cec",
        "f08b53eab79bd2d6ec7634fe002829d340ced578d4463339529070f7de0f424e",
    ),
    (
        1,
        "16d7675598dde69e12d64b9ee654ce5da2cb01ab2583c1dbf990a47ae64af91f",
        "05d3048aed253f00eeecd133cfa83d8e6b8c3bbb927efc4314bff5c4d200d27e",
    ),
    (
        2,
        "a54ab11ae309b5d5ddc2b7aec7b5a196bdacf7bb7086c73af257db32d18a15f5",
        "f1e674940eae7bd3c954990602191bfe8d612a6a13d05eaa36a8d100d14bf0d6",
    ),
    (
        3,
        "51e7a87bb544a7b5694aad30104c451abee20f417d0252e5cc4168f2b081d32d",
        "b9627159c0fb69d2657a3eb856b31da596e59dd582cef74f5fb2323d62d5856a",
    ),
    (
        4,
        "dd480014d1d6608adfff62eb458f906814de623cb72660cc168f967c33ee41e3",
        "14b554a7d725cdc746def0bd6126f633421279d330002f3821ba75b12973137a",
    ),
    (
        5,
        "18d297de81bcb0817074755f82434d701e8fa19420f3802cecf6c3a88c28315b",
        "0b82739911bac7a06073132c06df22018e7aa33ede9335c6420da8539d6a5c34",
    ),
    (
        6,
        "6e9406c597a24ec8a80633f95e189e15122cb80fe2b44dcc61be9af494f3f3ef",
        "ecd3ee1910d60e726730c445baac38babd602cb57783f8ba27d556ef0426a11d",
    ),
    (
        7,
        "f1dd78dde7ebe3e940cc70ebe3467c6c32459e995bc80c16495a2fdfc087bd93",
        "a308e02fad90ad878ef2dc7a1e39e9cb7fd420513e98c11bf06bf12a3ab5d784",
    ),
    (
        8,
        "ab9e4b4397e1ff67787fc792e1b801b5f51b7c863a7951d7928450ab5ac3349f",
        "2e83dd80699133e2823a3cb1e4cfd3b902158ff52e962c156e07fa36cf028a3a",
    ),
    (
        9,
        "59d3b658712ff9a0768b5f8f2459cd133fa3aabb4634d66dc73c224e14d27f96",
        "c2e1ca830ae26c1f2870d50a1aaed65b8e25acf5b5be50c46a214b215be0f66a",
    ),
    (
        10,
        "654bfc776e12c5ae37859dcf316d68206c83259b880aa7d54fea87cb7195a05a",
        "364ff1b70ef431ebb48e8e51dfd6d484330fae96d40f819b7a9fb15d9662808d",
    ),
    (
        11,
        "db5a10842cd5a79647350775aaf191af967fe19f2a685a59827f0213bdbe6c05",
        "536edc7825f94070df5b53595830aa55c609ef0e1b5cd35d12754a40cfb56f32",
    ),
    (
        12,
        "cf8527c3dbf641846c7e75e289b589345188e1739c7a0ae5362b7c765ebaf246",
        "571da5b302072895ae3285f41e9f0f9c7d64102a5c90a97de471e81eb7192940",
    ),
];

/// The SSZ-encoded checkpoint claim of each epoch of
/// [`build_v1_genesis_chain`] after genesis, in hex.
const V1_GENESIS_CLAIMS: &[&str] = &[
    "010000000000000000000000f08b53eab79bd2d6ec7634fe002829d340ced578d4463339529070f7de0f424e05000000000000000b82739911bac7a06073132c06df22018e7aa33ede9335c6420da8539d6a5c34688899b37f634e64a8cfc63ad1d6a3efa5f59ee4c184d48e8ce45196134d3f6d836ee68507cfa0581525db189c822c56cc14a3870b0a59420b49539a686c06f2ae3570fa4f589b1cdee4a1ba4d40031d61b9ac8d28f996bf753bf007fee381e088d07af770a0655458c488829ff35e2da808ef57b31797740746323372b858ef",
    "0200000005000000000000000b82739911bac7a06073132c06df22018e7aa33ede9335c6420da8539d6a5c3408000000000000002e83dd80699133e2823a3cb1e4cfd3b902158ff52e962c156e07fa36cf028a3ac7582cb6d142ae5bc8aef2a5c5c78bc82a27f02219e5677791b1d1ae89332cc7fcb910cd5a50de31b5b04bf3a0036fc3409ac3e8363e6a91639d1d91af5c4047816ff8d5775b73ff820d1ca7b3227ceab5049f4228032d698e3d1838fbe0910697682e09d5a830b37a2e8b67d680d71f0b61b302b9c0dee42ef0a09d9f33afab",
    "0300000008000000000000002e83dd80699133e2823a3cb1e4cfd3b902158ff52e962c156e07fa36cf028a3a0a00000000000000364ff1b70ef431ebb48e8e51dfd6d484330fae96d40f819b7a9fb15d9662808d0000000000000000000000000000000000000000000000000000000000000000baa3f152732b244305e104a6999be0bcd49e670768e3bb992435f46e3da2f8fde3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85537517f69baf8454f6dc074bca287f3d3acd87161403084ae18277462d9a8ef0f",
    "040000000a00000000000000364ff1b70ef431ebb48e8e51dfd6d484330fae96d40f819b7a9fb15d9662808d0c00000000000000571da5b302072895ae3285f41e9f0f9c7d64102a5c90a97de471e81eb71929407596fa4fa8b516a479e2706efebc4b82bdfc95ef8a65f84ac8fa0d7c9feb09aebaa3f152732b244305e104a6999be0bcd49e670768e3bb992435f46e3da2f8fde3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8557853bff92e5abba82fe3376479675ce2bcbfb8ebf6e584072ab8372471253fe1",
];

/// The blocks `v0.4.0-rc.1` produces for [`build_v0_genesis_chain`].
const V0_GENESIS_BLOCKS: &[GoldenBlock] = &[
    (
        0,
        "71576c6de7ecb2ce9bd9bd48dc93d15f4fc5b57355c8944d0ec037ee5661aecb",
        "928b7411c3d649c15a5c911439bb40646efa0e269252bd0d80567a6a37dd7b49",
    ),
    (
        1,
        "d8cf437b143113ab98159dc19233c5a421879ff583f801b94ebe0db7473acb26",
        "b028806dd51671b9d481242683d5267beff72307f47eeb902b033daee6942546",
    ),
    (
        2,
        "58c642ab39b16764d9498d21ec8088c766b1f45583cde5798b26c002d51e37f0",
        "c43db0da31c3669209a1e99491e9cdb0a00c7fecc517cdcfb5f432663de2f8f1",
    ),
    (
        3,
        "f48c9d4bb46acf70c4238f98ef2bf092d31ea11d9bd9b3ec1f7c595e8310b129",
        "2e38b64b3c0c796361f2aaaf65f3875bba6eb0b5b6b861da5053528f783ced51",
    ),
    (
        4,
        "866c04428913cb5be32bc0b7e6a3236e7f2a5c14ac4802aaf2859f541a388819",
        "8868b68823ceac545ef78a9f6f3a8c7d3de3cfdd59be0800ea867a2ec6ea3cc3",
    ),
];

/// The SSZ-encoded checkpoint claim of each epoch of
/// [`build_v0_genesis_chain`] after genesis, in hex.
const V0_GENESIS_CLAIMS: &[&str] = &[
    "010000000000000000000000928b7411c3d649c15a5c911439bb40646efa0e269252bd0d80567a6a37dd7b490200000000000000c43db0da31c3669209a1e99491e9cdb0a00c7fecc517cdcfb5f432663de2f8f10000000000000000000000000000000000000000000000000000000000000000baa3f152732b244305e104a6999be0bcd49e670768e3bb992435f46e3da2f8fde3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855f589d783b732b84adb3172dde23f013ed5dfa28eecd21e4cba9e318bc9d8a1a4",
    "020000000200000000000000c43db0da31c3669209a1e99491e9cdb0a00c7fecc517cdcfb5f432663de2f8f104000000000000008868b68823ceac545ef78a9f6f3a8c7d3de3cfdd59be0800ea867a2ec6ea3cc3787d8190dc7d296a16f08d8895ebe882b4938ee200e5086f0e04a0681468426d196cb04b49ab57757bc2822623c52e57091a1fb6f3d174e6dafd81b6dc1e94aee3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85594ac12376c5d9210aac9d81c9a88a17b728a8c19f09937a8c495c662b8dd97b4",
];

/// One epoch of a golden chain and the state it started from.
struct GoldenEpoch {
    pre_epoch_state: MemoryStateBaseLayer<OLStateV1>,
    previous_terminal: OLBlockHeaderV1,
    blocks: Vec<OLBlockV1>,
}

/// Builds a chain one block at a time. Genesis runs under the spec of the
/// pre-genesis state; every later block runs under [`OLSpecId::V1`].
struct GoldenChain {
    state: MemoryStateBaseLayer<OLStateV1>,
    runtime_params: OLRuntimeParams,
    genesis: OLBlockV1,
    epochs: Vec<GoldenEpoch>,
    current: Option<GoldenEpoch>,
}

impl GoldenChain {
    /// Executes an empty genesis block, as networks run it, on
    /// `pre_genesis_state` under the spec that state carries.
    fn new(pre_genesis_state: MemoryStateBaseLayer<OLStateV1>) -> Self {
        let genesis_spec = pre_genesis_state.spec_versions().cur_spec();
        let runtime_params = OLRuntimeParams::test_default();
        let mut state = pre_genesis_state;
        let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
        let output = construct_block(
            genesis_spec,
            &mut state,
            BlockContext::new(&genesis_info, None),
            BlockComponents::new_manifests(Vec::new()).as_terminal(),
            &runtime_params,
        )
        .expect("golden genesis constructs");
        Self {
            state,
            runtime_params,
            genesis: to_ol_block(output.completed_block()),
            epochs: Vec::new(),
            current: None,
        }
    }

    fn parent(&self) -> &OLBlockHeaderV1 {
        self.current
            .as_ref()
            .and_then(|epoch| epoch.blocks.last())
            .or_else(|| self.epochs.last().and_then(|epoch| epoch.blocks.last()))
            .unwrap_or(&self.genesis)
            .header()
    }

    /// Returns the L1 height of the next manifest the chain can process.
    fn next_l1_height(&self) -> u32 {
        self.state.last_l1_height() + 1
    }

    fn push(&mut self, components: BlockComponents) {
        let parent = self.parent().clone();
        let epoch = self.current.get_or_insert_with(|| GoldenEpoch {
            pre_epoch_state: self.state.clone(),
            previous_terminal: parent.clone(),
            blocks: Vec::new(),
        });
        let slot = parent.slot() + 1;
        let block_info = BlockInfo::new(
            EPOCH_RUNNER_GENESIS_TIMESTAMP + slot * EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
            slot,
            parent.epoch() + u32::from(parent.is_terminal()),
        );
        let is_terminal = components.is_terminal();
        let output = construct_block(
            OLSpecId::V1,
            &mut self.state,
            BlockContext::new(&block_info, Some(&parent)),
            components,
            &self.runtime_params,
        )
        .expect("golden block constructs");
        epoch.blocks.push(to_ol_block(output.completed_block()));
        if is_terminal {
            self.epochs
                .push(self.current.take().expect("current epoch was just set"));
        }
    }

    fn push_txs(&mut self, txs: Vec<OLTransactionV1>) {
        self.push(BlockComponents::new_txs_from_ol_transactions(txs));
    }

    fn push_manifest(&mut self, manifest: AsmManifest, is_terminal: bool) {
        self.push(BlockComponents::new_manifests(vec![manifest]).with_terminal(is_terminal));
    }

    fn blocks(&self) -> impl Iterator<Item = &OLBlockV1> {
        assert!(
            self.current.is_none(),
            "chain must end at an epoch terminal"
        );
        iter::once(&self.genesis).chain(self.epochs.iter().flat_map(|epoch| &epoch.blocks))
    }
}

fn gam_tx(target_index: u32, payload: &[u8]) -> OLTransactionV1 {
    OLTransactionV1::new(
        OLTransactionDataV1::from_gam_bytes(make_account_id(target_index), payload.to_vec())
            .expect("GAM payload fits"),
        TxProofsV1::new_empty(),
    )
}

fn ee_key_update_manifest(height: u32, serial: AccountSerial) -> AsmManifest {
    let log = AsmLogEntry::from_log(&EePredicateKeyUpdate::new(
        serial,
        PredicateKey::always_accept(),
    ))
    .expect("key update log encodes");
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_log(log)
        .build()
}

/// Builds the snark account update that consumes inbox message `index` of
/// `msgs`, with `effect` applied to its builder.
fn snark_update(
    state: &MemoryStateBaseLayer<OLStateV1>,
    msgs: &[MessageEntry],
    index: usize,
    effect: impl FnOnce(SnarkUpdateBuilder) -> SnarkUpdateBuilder,
) -> OLTransactionV1 {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let mut tracker = InboxMmrTracker::new();
    for msg in msgs {
        tracker.add_message(msg);
    }
    let (_, snark_state) = get_snark_state_expect(state, snark_id);
    let builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
        .with_processed_msgs(vec![msgs[index].clone()])
        .with_inbox_proofs(vec![tracker.proof_for(index)]);
    effect(builder).build(snark_id, make_state_root(index as u8 + 2), vec![0u8; 32])
}

/// Builds four V1 epochs on a V1 genesis:
///
/// 1. two inbox deliveries, a snark account update with a transfer, a manifest in a non-terminal
///    block, and a deposit to the snark account at the terminal;
/// 2. a snark account update with a withdrawal, a message to an empty account, and an EE predicate
///    key update at the terminal;
/// 3. no manifests, so the epoch ends on the previous epoch's last L1 block;
/// 4. a message, and a checkpoint predicate enactment at the terminal.
fn build_v1_genesis_chain() -> GoldenChain {
    let mut pre_genesis_state = make_genesis_state();
    let snark_serial = epoch_runner_seed_accounts(&mut pre_genesis_state);
    let mut chain = GoldenChain::new(pre_genesis_state);
    let msgs = [
        snark_inbox_msg_with_data(b"golden msg 0"),
        snark_inbox_msg_with_data(b"golden msg 1"),
    ];

    // Epoch 1.
    for msg in &msgs {
        chain.push_txs(vec![gam_tx(TEST_SNARK_ACCOUNT_ID, msg.payload().data())]);
    }
    let update = snark_update(&chain.state, &msgs, 0, |builder| {
        builder.with_transfer(make_account_id(TEST_RECIPIENT_ID), 1_000_000)
    });
    chain.push_txs(vec![update]);
    chain.push_manifest(make_empty_manifest(chain.next_l1_height(), 1), false);
    let deposit = make_deposit_manifest_for_account(
        chain.next_l1_height(),
        2,
        snark_serial,
        SubjectId::from([7u8; 32]),
        BitcoinAmount::try_from(150_000_000).expect("amount below the money supply"),
    );
    chain.push_manifest(deposit, true);

    // Epoch 2.
    let update = snark_update(&chain.state, &msgs, 1, |builder| {
        builder.with_output_message(
            BRIDGE_GATEWAY_ACCT_ID,
            100_000_000,
            make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x14)),
        )
    });
    chain.push_txs(vec![update]);
    chain.push_txs(vec![gam_tx(TEST_RECIPIENT_ID, b"to recipient")]);
    chain.push_manifest(
        ee_key_update_manifest(chain.next_l1_height(), snark_serial),
        true,
    );

    // Epoch 3: no manifests.
    chain.push(BlockComponents::new_empty());
    chain.push(BlockComponents::new_empty().as_terminal());

    // Epoch 4: ends at a checkpoint predicate enactment.
    chain.push_txs(vec![gam_tx(TEST_RECIPIENT_ID, b"before the enactment")]);
    chain.push_manifest(
        make_checkpoint_predicate_enactment_manifest(chain.next_l1_height(), 1),
        true,
    );

    chain
}

/// Builds two V1 epochs on a V0 genesis, as on a network launched on 0.3.0:
/// the first V1 block wraps the V0 state. The first epoch processes no
/// manifests; the second carries a message and a deposit.
fn build_v0_genesis_chain() -> GoldenChain {
    let mut pre_genesis_state = make_genesis_state();
    let snark_serial = epoch_runner_seed_accounts(&mut pre_genesis_state);
    pre_genesis_state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
    let mut chain = GoldenChain::new(pre_genesis_state);

    // Epoch 1: wraps the V0 state, processes no manifests.
    chain.push_txs(vec![gam_tx(TEST_RECIPIENT_ID, b"first V1 epoch")]);
    chain.push(BlockComponents::new_empty().as_terminal());

    // Epoch 2.
    chain.push_txs(vec![gam_tx(TEST_SNARK_ACCOUNT_ID, b"second V1 epoch")]);
    let deposit = make_deposit_manifest_for_account(
        chain.next_l1_height(),
        3,
        snark_serial,
        SubjectId::from([9u8; 32]),
        BitcoinAmount::try_from(250_000_000).expect("amount below the money supply"),
    );
    chain.push_manifest(deposit, true);

    chain
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Checks `chain` against the golden blocks and claims, and that block
/// verification and DA replay reproduce it.
fn assert_matches_golden(
    chain: &GoldenChain,
    golden_blocks: &[GoldenBlock],
    golden_claims: &[&str],
) {
    // Block verification reproduces construction after genesis. Genesis has
    // no parent to verify against, and V0 rules only construct it.
    let first_epoch = chain
        .epochs
        .first()
        .expect("chain has an epoch after genesis");
    let mut state = first_epoch.pre_epoch_state.clone();
    let mut parent = chain.genesis.header();
    for block in chain.epochs.iter().flat_map(|epoch| &epoch.blocks) {
        verify_block(
            OLSpecId::V1,
            &mut state,
            block.header(),
            Some(parent),
            block.body(),
            &chain.runtime_params,
        )
        .expect("golden block verifies");
        parent = block.header();
    }

    let blocks: Vec<(u64, String, String)> = chain
        .blocks()
        .map(|block| {
            let header = block.header();
            (
                header.slot(),
                to_hex(header.state_root().as_ref()),
                to_hex(header.compute_blkid().as_ref()),
            )
        })
        .collect();
    let expected_blocks: Vec<(u64, String, String)> = golden_blocks
        .iter()
        .map(|&(slot, root, blkid)| (slot, root.to_owned(), blkid.to_owned()))
        .collect();
    assert_eq!(blocks, expected_blocks, "blocks differ from v0.4.0-rc.1");

    let mut claims = Vec::new();
    for epoch in &chain.epochs {
        let terminal = epoch.blocks.last().expect("epoch has a terminal").header();
        let (da_state_diff_bytes, _) = compute_epoch_da(
            OLSpecId::V1,
            epoch.pre_epoch_state.clone(),
            &epoch.blocks,
            &epoch.previous_terminal,
            &chain.runtime_params,
        )
        .expect("epoch DA computes")
        .into_parts();

        // DA replay reproduces the terminal root.
        let manifests: Vec<AsmManifest> = epoch
            .blocks
            .iter()
            .filter_map(|block| block.body().manifests())
            .flat_map(|container| container.manifests().iter().cloned())
            .collect();
        let mut replayed_state = epoch.pre_epoch_state.clone();
        apply_da_epoch(
            OLSpecId::V1,
            &mut replayed_state,
            &EpochInfo::new(
                BlockInfo::from_header(terminal),
                epoch.previous_terminal.compute_block_commitment(),
            ),
            &da_state_diff_bytes,
            &manifests,
            &chain.runtime_params,
        )
        .expect("epoch DA applies");
        assert_eq!(
            replayed_state.compute_state_root().expect("state root"),
            *terminal.state_root(),
            "epoch {} DA replay root",
            terminal.epoch()
        );

        let input = CheckpointProverInput {
            start_state: epoch.pre_epoch_state.to_container(),
            blocks: epoch.blocks.clone(),
            parent: epoch.previous_terminal.clone(),
            da_state_diff_bytes,
        };
        let claim = CheckpointProgram::execute(&input, OLSpecId::V1, chain.runtime_params)
            .expect("checkpoint program proves the epoch");
        claims.push(to_hex(&claim.as_ssz_bytes()));
    }
    assert_eq!(
        claims, golden_claims,
        "checkpoint claims differ from v0.4.0-rc.1"
    );
}

#[test]
fn test_v1_genesis_chain_matches_v0_4_0_rc_1() {
    let chain = build_v1_genesis_chain();
    assert_eq!(chain.epochs.len(), 4);
    assert_matches_golden(&chain, V1_GENESIS_BLOCKS, V1_GENESIS_CLAIMS);
}

#[test]
fn test_v0_genesis_chain_matches_v0_4_0_rc_1() {
    let chain = build_v0_genesis_chain();
    assert_eq!(chain.epochs.len(), 2);
    assert_eq!(
        chain.epochs[0].pre_epoch_state.spec_versions(),
        OLSpecVersions::uniform(OLSpecId::V0),
        "the first V1 epoch starts from the V0 genesis state"
    );
    assert_matches_golden(&chain, V0_GENESIS_BLOCKS, V0_GENESIS_CLAIMS);
}
