//! # Debug ASM Specification
//!
//! This crate provides the Debug ASM specification for the Strata protocol.
//! The Debug ASM spec wraps the regular ASM spec and adds debug capabilities for testing.

use strata_asm_common::{AnchorState, AsmSpec, SectionState, Stage, Subprotocol};
use strata_asm_params::AsmParams;
use strata_asm_proto_debug_v1::DebugSubproto;
use strata_asm_spec::{StrataAsmSpec, construct_genesis_state};

/// Debug ASM specification that includes the debug subprotocol.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebugAsmSpec {
    inner: StrataAsmSpec,
}

impl AsmSpec for DebugAsmSpec {
    type Params = AsmParams;

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        stage.invoke_subprotocol::<DebugSubproto>();
        self.inner.call_subprotocols(stage);
    }

    fn construct_genesis_state(&self, params: &Self::Params) -> AnchorState {
        construct_debug_genesis_state(params)
    }
}

impl DebugAsmSpec {
    /// Creates a debug ASM spec by wrapping a production spec.
    pub fn new(inner: StrataAsmSpec) -> Self {
        Self { inner }
    }

    /// Builds the debug spec from params.
    pub fn from_asm_params(params: &AsmParams) -> Self {
        Self {
            inner: StrataAsmSpec::from_asm_params(params),
        }
    }
}

/// Builds the genesis [`AnchorState`] for the debug spec.
pub fn construct_debug_genesis_state(params: &AsmParams) -> AnchorState {
    let mut state = construct_genesis_state(params);

    let debug_state = DebugSubproto::init(&());
    let debug_section = SectionState::from_state::<DebugSubproto>(&debug_state)
        .expect("asm: Debug subprotocol genesis state fits section data capacity");

    let mut sections: Vec<_> = state.sections.to_vec();
    sections.insert(0, debug_section);
    state.sections = sections
        .try_into()
        .expect("asm: genesis sections fit within capacity");

    state
}
