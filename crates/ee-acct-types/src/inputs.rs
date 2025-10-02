//! Types for block processing.

use crate::traits::{ExecBlock, ExecHeader};

/// Execution payload we process a block with.
///
/// This is the parts of a block that we use as an input to processing, but not
/// the parts that we check against things in the header with.
///
/// This only contains things *in* the block, it does not include outside
/// inputs into the block which are specified separately (even if they do
/// technically exist in some form within the block, as long as we don't
/// directly check those).
pub struct ExecPayload<'b, EB: ExecBlock> {
    header_intrinsics: &'b <EB::Header as ExecHeader>::Intrinsics,
    body: &'b EB::Body,
}

impl<'b, EB: ExecBlock> ExecPayload<'b, EB> {
    pub fn new(
        header_intrinsics: &'b <EB::Header as ExecHeader>::Intrinsics,
        body: &'b EB::Body,
    ) -> Self {
        Self {
            header_intrinsics,
            body,
        }
    }

    pub fn header_intrinsics(&self) -> &'b <<EB as ExecBlock>::Header as ExecHeader>::Intrinsics {
        self.header_intrinsics
    }

    pub fn body(&self) -> &'b <EB as ExecBlock>::Body {
        self.body
    }
}
