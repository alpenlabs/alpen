//! Fallible extraction of mandatory values from the extensible state wire format.

use ssz::DecodeError;
use ssz_types::Optional;

pub(crate) fn require_present<T>(value: Optional<T>, field: &str) -> Result<T, DecodeError> {
    match value {
        Optional::Some(value) => Ok(value),
        Optional::None => Err(DecodeError::BytesInvalid(format!(
            "missing required OL state field: {field}"
        ))),
    }
}
