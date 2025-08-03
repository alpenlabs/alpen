use strata_asm_common::{AsmSpec, GenesisProvider, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1GenesisConfig, BridgeV1Subproto};
use strata_asm_proto_core::{CoreGenesisConfig, OLCoreSubproto};
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use zkaleido::VerifyingKey;

/// ASM spec for the Strata protocol.
#[derive(Debug)]
pub struct StrataAsmSpec;

impl AsmSpec for StrataAsmSpec {
    const MAGIC_BYTES: strata_l1_txfmt::MagicBytes = *b"ALPN";

    fn genesis_config_for<S: strata_asm_common::Subprotocol>() -> S::GenesisConfig
    where
        Self: GenesisProvider<S>,
    {
        <Self as GenesisProvider<S>>::genesis_config()
    }

    fn call_subprotocols(stage: &mut impl Stage<Self>) {
        stage.process_subprotocol::<OLCoreSubproto>();
        stage.process_subprotocol::<BridgeV1Subproto>();
    }
}

// Implement GenesisProvider for each subprotocol
impl GenesisProvider<OLCoreSubproto> for StrataAsmSpec {
    fn genesis_config() -> CoreGenesisConfig {
        // TODO: These should be replaced with actual genesis values
        // For now, using placeholder values to make the code compile
        CoreGenesisConfig::new(
            VerifyingKey::default(), // TODO: Replace with actual verifying key
            L1BlockCommitment::new(
                0, // TODO: Replace with actual genesis block height
                strata_primitives::l1::L1BlockId::default()
            ), // TODO: Replace with actual genesis L1 block
            Buf32::zero(), // TODO: Replace with actual sequencer pubkey
        ).expect("Failed to create CoreGenesisConfig")
    }
}

impl GenesisProvider<BridgeV1Subproto> for StrataAsmSpec {
    fn genesis_config() -> BridgeV1GenesisConfig {
        BridgeV1GenesisConfig::default()
    }
}
