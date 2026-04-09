use strata_asm_common::AsmLog;
use strata_codec::Codec;
use strata_codec_utils::CodecSsz;
use strata_identifiers::AccountSerial;
use strata_msg_fmt::TypeId;
use strata_predicate::PredicateKey;

use crate::constants::EE_PREDICATE_KEY_UPDATE_LOG_TYPE;

/// Records an update to a snark account's `update_vk` (predicate key) used to
/// verify future updates to that account.
///
/// The target account is identified by its [`AccountSerial`]; the OL STF
/// resolves the serial and applies the new predicate key during manifest
/// processing.
#[derive(Debug, Clone, Codec)]
pub struct EePredicateKeyUpdate {
    /// Serial of the snark account whose predicate key is being updated.
    account: AccountSerial,

    /// New predicate key to install on the target account.
    new_predicate: CodecSsz<PredicateKey>,
}

impl EePredicateKeyUpdate {
    /// Creates a new [`EePredicateKeyUpdate`] for the given account serial and
    /// predicate key.
    pub fn new(account: AccountSerial, new_predicate: PredicateKey) -> Self {
        Self {
            account,
            new_predicate: CodecSsz::new(new_predicate),
        }
    }

    /// Returns the target account serial.
    pub fn account(&self) -> AccountSerial {
        self.account
    }

    /// Returns a reference to the new predicate key.
    pub fn new_predicate(&self) -> &PredicateKey {
        self.new_predicate.inner()
    }

    /// Consumes this log and returns the owned predicate key.
    pub fn into_new_predicate(self) -> PredicateKey {
        self.new_predicate.into_inner()
    }
}

impl AsmLog for EePredicateKeyUpdate {
    const TY: TypeId = EE_PREDICATE_KEY_UPDATE_LOG_TYPE;
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::AccountSerial;
    use strata_predicate::PredicateKey;

    use super::*;

    #[test]
    fn ee_predicate_key_update_roundtrip() {
        let account = AccountSerial::new(42);
        let new_predicate = PredicateKey::always_accept();
        let update = EePredicateKeyUpdate::new(account, new_predicate.clone());

        let encoded = encode_to_vec(&update).expect("encoding should not fail");
        let decoded: EePredicateKeyUpdate =
            decode_buf_exact(&encoded).expect("decoding should not fail");

        assert_eq!(decoded.account(), account);
        assert_eq!(decoded.new_predicate(), &new_predicate);
    }

    #[test]
    fn ee_predicate_key_update_type_id() {
        assert_eq!(
            EePredicateKeyUpdate::TY,
            EE_PREDICATE_KEY_UPDATE_LOG_TYPE,
            "type ID must match the constant"
        );
    }
}
