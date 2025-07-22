//! Defines the [`BridgeTxState`] type that tracks the state of signature collection for a
//! particular [`Psbt`](bitcoin::Psbt).

use std::{collections::BTreeMap, ops::Not};

use arbitrary::Arbitrary;
use bitcoin::{Transaction, TxOut, Txid};
use borsh::{BorshDeserialize, BorshSerialize};
use musig2::{PartialSignature, PubNonce};
use strata_primitives::{
    bridge::{
        Musig2PartialSig, Musig2PubNonce, Musig2SecNonce, OperatorIdx, OperatorPartialSig,
        PublickeyTable, TxSigningData,
    },
    l1::{BitcoinPsbt, TaprootSpendPath},
};

use super::errors::{BridgeTxStateError, EntityResult};

/// The state a transaction is in with respect to the number of signatures that have been collected
/// from the bridge federation signatories.
#[derive(Debug, Clone, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize)]
pub struct BridgeTxState {
    /// The partially signed bitcoin transaction that this state tracks.
    psbt: BitcoinPsbt,

    /// The witness elements required to spend a taproot output.
    spend_path: TaprootSpendPath,

    /// The table of pubkeys that is used to lock the UTXO present as an input in the psbt.
    /// This table maps the [`OperatorIdx`] to their corresponding pubkeys.
    pubkey_table: PublickeyTable,

    /// The private nonce unique to the transaction being tracked by this state.
    // FIXME: storing the secret nonce in the db is not a good practice but since it needs to be
    // generated uniquely per transaction that is to be signed, we store it here.
    // For more on nonce security, see [this](https://docs.rs/musig2/latest/musig2/#security).
    secnonce: Musig2SecNonce,

    /// The (public) nonces shared for the particular [`Psbt`](bitcoin::Psbt)
    /// that this state tracks under MuSig2.
    collected_nonces: BTreeMap<OperatorIdx, Musig2PubNonce>,

    /// The table of partial signatures collected so far per operator for the first input in
    /// [`Self::psbt`].
    collected_sigs: BTreeMap<OperatorIdx, Musig2PartialSig>,
}

impl BridgeTxState {
    /// Create a new [`BridgeTxState`] for the given [`Psbt`](bitcoin::Psbt)
    /// and list of [`bitcoin::secp256k1::PublicKey`].
    pub fn new(
        tx_signing_data: TxSigningData,
        pubkey_table: PublickeyTable,
        sec_nonce: Musig2SecNonce,
    ) -> EntityResult<Self> {
        let collected_sigs = BTreeMap::new();

        let collected_nonces: BTreeMap<OperatorIdx, Musig2PubNonce> = BTreeMap::new();

        Ok(Self {
            psbt: tx_signing_data.psbt,
            spend_path: tx_signing_data.spend_path,
            pubkey_table,
            secnonce: sec_nonce,
            collected_nonces,
            collected_sigs,
        })
    }

    /// Get the [`Psbt`](bitcoin::Psbt) that this state is associated with.
    pub fn psbt(&self) -> &BitcoinPsbt {
        &self.psbt
    }

    /// Get the spend info associated with each input in the [`Psbt`](bitcoin::Psbt).
    pub fn spend_path(&self) -> &TaprootSpendPath {
        &self.spend_path
    }

    /// Get the relevant previous outputs of the [`Psbt`](bitcoin::Psbt)
    /// that this state tracks.
    pub fn prevouts(&self) -> Vec<TxOut> {
        self.psbt()
            .inner()
            .inputs
            .iter()
            .map(|input| {
                input
                    .witness_utxo
                    .clone()
                    .expect("witness UTXO must be present")
            })
            .collect()
    }

    /// Get the number of required signatures for the [`Psbt`](bitcoin::Psbt)
    /// to be considered fully signed.
    pub fn required_signatures(&self) -> usize {
        self.pubkey_table.0.keys().len()
    }

    /// Get the private nonce for the transaction being tracked in this state.
    pub fn secnonce(&self) -> &Musig2SecNonce {
        &self.secnonce
    }

    /// Get the [`PublickeyTable`] that maps [`OperatorIdx`] to the corresponding `PublicKey`
    /// correspondng to the multisig.
    pub fn pubkeys(&self) -> &PublickeyTable {
        &self.pubkey_table
    }

    /// Get the unsigned transaction from the [`Psbt`](bitcoin::Psbt).
    pub fn unsigned_tx(&self) -> &Transaction {
        &self.psbt().inner().unsigned_tx
    }

    /// Compute the Transaction ID for the [`Psbt`](bitcoin::Psbt) that this state tracks.
    pub fn compute_txid(&self) -> Txid {
        self.psbt.compute_txid()
    }

    /// Get the map of collected nonces.
    pub fn collected_nonces(&self) -> &BTreeMap<OperatorIdx, Musig2PubNonce> {
        &self.collected_nonces
    }

    /// Get table of signatures collected so far per input in the transaction.
    pub fn collected_sigs(&self) -> &BTreeMap<OperatorIdx, Musig2PartialSig> {
        &self.collected_sigs
    }

    /// Get the ordered list of nonces collected so far.
    // NOTE: As accessing the list of nonces is usually done to `sum` them up, it's convenient to
    // return an iterator over the inner `PubNonce` type.
    pub fn ordered_nonces(&self) -> impl IntoIterator<Item = PubNonce> + '_ {
        self.collected_nonces().values().map(|v| v.inner().clone())
    }

    /// Check if all the nonces have been received.
    pub fn has_all_nonces(&self) -> bool {
        // Since we only add valid nonces (by checking the pubkey table), just checking the length
        // should be sufficient.
        self.collected_nonces.keys().len() == self.required_signatures()
    }

    /// Add a nonce to the collected nonces.
    ///
    /// # Returns
    ///
    /// A flag indicating whether adding this nonce completes the collection.
    pub fn add_nonce(
        &mut self,
        operator_index: &OperatorIdx,
        nonce: Musig2PubNonce,
    ) -> EntityResult<bool> {
        if self.pubkey_table.0.contains_key(operator_index).not() {
            return Err(BridgeTxStateError::Unauthorized)?;
        }

        self.collected_nonces.insert(*operator_index, nonce);

        Ok(self.has_all_nonces())
    }

    /// Check if all the required signatures have been collected for the
    /// [`Psbt`](bitcoin::Psbt).
    pub fn is_fully_signed(&self) -> bool {
        // for each input, check all signatures have been collected
        // each signature is added only if the signer is part of the `pubkey_table`,
        // so checking the total number of signatures so far suffices.
        self.collected_sigs.values().len() == self.required_signatures()
    }

    /// Add a signature to the collection. If the signature corresponding to a particular pubkey has
    /// already been added, it is updated.
    ///
    /// **Note**: This being a database-related operation, no validation is performed on the
    /// provided signature as that requires access to a signing module. The only validation that
    /// this method performs is that the signature comes from an [`OperatorIdx`] that is part of
    /// the `Self::pubkey_table`. It is assumed that all necessary validation has already been
    /// performed at the callsite.
    ///
    /// # Returns
    ///
    /// A boolean flag indicating whether the added signature completes the set of required
    /// signatures.
    ///
    /// # Errors
    ///
    /// If the [`OperatorPartialSig::signer_index`] is not a part of the required signatories or the
    /// `input_index` is not part of the [`Psbt`](bitcoin::Psbt).
    pub fn add_signature(&mut self, signature_info: OperatorPartialSig) -> EntityResult<bool> {
        // Some extra validation (should also be done by the rollup node)
        // Check if the signer is authorized i.e., they are part of the federation.
        let signer_index = signature_info.signer_index();

        if self.pubkey_table.0.contains_key(signer_index).not() {
            return Err(BridgeTxStateError::Unauthorized)?;
        }

        self.collected_sigs
            .insert(*signer_index, *signature_info.signature());

        Ok(self.is_fully_signed())
    }

    /// Get the ordered signatures per input collected so far.
    pub fn ordered_sigs(&self) -> impl Iterator<Item = PartialSignature> + '_ {
        self.collected_sigs.values().map(|v| *v.inner())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use arbitrary::Unstructured;
    use strata_test_utils::bridge::{
        generate_keypairs, generate_mock_tx_signing_data, generate_pubkey_table,
        generate_sec_nonce, permute,
    };

    use super::*;
    use crate::entities::errors::EntityError;

    #[test]
    fn test_has_all_nonces() {
        let own_index = 0;
        let num_operators = 2;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        assert!(
            !tx_state.has_all_nonces(),
            "expected: false since no nonces have been collected but got: true"
        );

        let data = vec![0u8; 1024];
        let mut unstructured = Unstructured::new(&data[..]);

        for i in 0..num_operators {
            let random_nonce = Musig2PubNonce::arbitrary(&mut unstructured)
                .expect("should produce random pubnonce");

            tx_state
                .add_nonce(&(i as OperatorIdx), random_nonce)
                .expect("should be able to add nonce");
        }

        assert!(
            tx_state.has_all_nonces(),
            "expected: true but got: false for the collected nonces: {:?}",
            tx_state.collected_nonces()
        );
    }

    #[test]
    fn test_add_nonce() {
        let own_index = 0;
        let num_operators = 2;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let data = vec![0u8; 1024];
        let mut unstructured = Unstructured::new(&data[..]);

        for i in 0..num_operators {
            let random_nonce = Musig2PubNonce::arbitrary(&mut unstructured)
                .expect("should produce random pubnonce");

            let result = tx_state.add_nonce(&(i as OperatorIdx), random_nonce);

            assert!(result.is_ok(), "operator {i} should exist");

            if (i + 1) < num_operators {
                assert!(!result.unwrap(), "should not have all nonces");
            } else {
                assert!(result.unwrap(), "should have all nonces");
            }
        }

        let random_nonce =
            Musig2PubNonce::arbitrary(&mut unstructured).expect("should produce random pubnonce");
        let result = tx_state.add_nonce(&(num_operators as OperatorIdx), random_nonce);

        assert!(
            result.is_err_and(|e| matches!(e, EntityError::BridgeTxState(BridgeTxStateError::Unauthorized))),
            "should result in `BridgeOpUnauthorized` error when adding nonce from an operator that is not part of the federation");
    }

    #[test]
    fn test_ordered_nonces() {
        let own_index = 0;
        let num_operators = 10;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let data = vec![0u8; 1024];
        let mut unstructured = Unstructured::new(&data[..]);

        let mut operator_ids = (0..num_operators)
            .map(|v| v as OperatorIdx)
            .collect::<Vec<OperatorIdx>>();
        permute(&mut operator_ids);

        let mut nonce_table: BTreeMap<OperatorIdx, PubNonce> = BTreeMap::new();
        for operator_idx in operator_ids {
            let operator_idx = operator_idx as OperatorIdx;

            let random_nonce = Musig2PubNonce::arbitrary(&mut unstructured)
                .expect("should produce random pubnonce");

            nonce_table.insert(operator_idx, random_nonce.inner().clone());
            let result = tx_state.add_nonce(&operator_idx, random_nonce);

            assert!(result.is_ok(), "operator {operator_idx} should exist");
        }

        let ordered_nonces = tx_state
            .ordered_nonces()
            .into_iter()
            .collect::<Vec<PubNonce>>();

        // this is more readable as we are iterating over operator indexes
        #[allow(clippy::needless_range_loop)]
        for i in 0..num_operators {
            // order in the pubkey table
            assert_eq!(
                ordered_nonces[i],
                tx_state
                    .collected_nonces()
                    .get(&(i as OperatorIdx))
                    .unwrap()
                    .inner()
                    .clone(),
                "nonces not ordered, mismatch for index: {i}"
            );
        }
    }

    #[test]
    fn test_is_fully_signed_all_signatures_present() {
        let own_index = 0;
        let num_operators = 2;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        for i in 0..num_operators {
            let data = vec![0u8; 32];
            let mut unstructured = Unstructured::new(&data);
            let sig = Musig2PartialSig::arbitrary(&mut unstructured)
                .expect("should generate arbitrary signature");

            tx_state
                .add_signature(OperatorPartialSig::new(sig, i as OperatorIdx))
                .unwrap();
        }

        assert!(
            tx_state.is_fully_signed(),
            "Expected transaction to be fully signed"
        );
    }

    #[test]
    fn test_is_fully_signed_missing_signature() {
        let own_index = 0;
        let num_operators = 1;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let data = vec![0u8; 32];
        let mut unstructured = Unstructured::new(&data);
        let sig = Musig2PartialSig::arbitrary(&mut unstructured)
            .expect("should generate arbitrary signature");

        tx_state
            .add_signature(OperatorPartialSig::new(sig, 0 as OperatorIdx))
            .unwrap();

        assert!(
            tx_state.is_fully_signed(),
            "Expected transaction to be fully signed"
        );

        // Remove the signature and test again
        tx_state.collected_sigs.remove(&(own_index as OperatorIdx));
        assert!(
            !tx_state.is_fully_signed(),
            "Expected transaction to not be fully signed"
        );
    }

    #[test]
    fn test_add_signature_success() {
        let own_index = 0;
        let num_operators = 1;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let data = vec![0u8; 32];
        let mut unstructured = Unstructured::new(&data);
        let sig = Musig2PartialSig::arbitrary(&mut unstructured)
            .expect("should generate arbitrary signature");

        assert!(tx_state
            .add_signature(OperatorPartialSig::new(sig, own_index as OperatorIdx),)
            .is_ok());

        assert_eq!(
            tx_state.collected_sigs.get(&(own_index as OperatorIdx)),
            Some(sig).as_ref()
        );
    }

    #[test]
    fn test_add_signature_invalid_pubkey() {
        let own_index = 0;
        let num_operators = 1;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let data = vec![0u8; 32];
        let mut unstructured = Unstructured::new(&data);
        let sig = Musig2PartialSig::arbitrary(&mut unstructured)
            .expect("should generate arbitrary signature");

        let unauthorized_signer_index = num_operators + 1;
        let result = tx_state.add_signature(OperatorPartialSig::new(
            sig,
            unauthorized_signer_index as OperatorIdx,
        ));
        assert!(result.is_err());

        assert!(matches!(
            result.unwrap_err(),
            EntityError::BridgeTxState(BridgeTxStateError::Unauthorized),
        ));
    }

    #[test]
    fn test_ordered_sigs() {
        let own_index = 0;
        let num_operators = 1;
        let mut tx_state = create_mock_tx_state(own_index, num_operators);

        let mut operator_ids = (0..num_operators).collect::<Vec<usize>>();
        permute(&mut operator_ids);

        for operator_id in operator_ids.clone() {
            let data = vec![0u8; 32];
            let mut unstructured = Unstructured::new(&data);
            let sig = Musig2PartialSig::arbitrary(&mut unstructured)
                .expect("should generate arbitrary signature");

            assert!(tx_state
                .add_signature(OperatorPartialSig::new(sig, operator_id as OperatorIdx),)
                .is_ok());
        }

        for (i, sig) in tx_state.ordered_sigs().enumerate().take(num_operators) {
            assert_eq!(
                sig,
                *tx_state
                    .collected_sigs()
                    .get(&(i as OperatorIdx))
                    .expect("signature for operator must exist in the collection")
                    .inner(),
                "ordered sigs should be... ordered, mismatch for ({i})"
            );
        }
    }

    /// Creates a mock [`BridgeTxState`] for the given params. We do this manually here instead of
    /// leveraging [`arbitrary::Arbitrary`] since we want more fine-grained control over the created
    /// structure.
    fn create_mock_tx_state(own_index: usize, num_operators: usize) -> BridgeTxState {
        let (pks, sks) = generate_keypairs(num_operators);

        let tx_output = generate_mock_tx_signing_data(false);

        let pubkey_table = generate_pubkey_table(&pks);

        let sec_nonce =
            generate_sec_nonce(&tx_output.psbt.compute_txid(), pks, sks[own_index], false);

        BridgeTxState::new(tx_output, pubkey_table, sec_nonce.into())
            .expect("Failed to create TxState")
    }
}
