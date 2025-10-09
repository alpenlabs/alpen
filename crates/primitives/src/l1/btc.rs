// Re-export Bitcoin types from strata-btc-types
pub use strata_btc_types::{
    BitcoinAddress, BitcoinPsbt, BitcoinScriptBuf, BitcoinTxOut, BitcoinTxid, Outpoint,
    TaprootSpendPath, XOnlyPk,
};
// Re-export from identifiers
#[rustfmt::skip]
pub use strata_identifiers::BitcoinAmount;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use arbitrary::{Arbitrary, Unstructured};
    use bitcoin::{
        hashes::Hash,
        key::Keypair,
        opcodes::all::OP_CHECKSIG,
        script::Builder,
        secp256k1::{Parity, SecretKey, SECP256K1},
        taproot::{ControlBlock, LeafVersion, TaprootBuilder, TaprootMerkleBranch},
        Address, Amount, Network, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    };
    use bitcoin_bosd::DescriptorType;
    use rand::{rngs::OsRng, Rng};
    use strata_test_utils::ArbitraryGenerator;

    use super::{
        BitcoinAddress, BitcoinAmount, BitcoinScriptBuf, BitcoinTxOut, BitcoinTxid,
        BorshDeserialize, BorshSerialize, XOnlyPk,
    };
    use crate::{
        buf::Buf32,
        errors::ParseError,
        l1::{BitcoinPsbt, TaprootSpendPath},
    };

    #[test]
    fn test_parse_bitcoin_address_network() {
        let possible_networks = [
            // mainnet
            Network::Bitcoin,
            // testnets
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];

        let num_possible_networks = possible_networks.len();

        let (secret_key, _) = SECP256K1.generate_keypair(&mut OsRng);
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        for network in possible_networks.iter() {
            // NOTE: only checking for P2TR addresses for now as those are the ones we use. Other
            // typs of addresses can also be checked but that shouldn't be necessary.
            let address = Address::p2tr(SECP256K1, internal_key, None, *network);
            let address_str = address.to_string();

            BitcoinAddress::parse(&address_str, *network).expect("address should parse");

            let invalid_network = match network {
                Network::Bitcoin => {
                    // get one of the testnets
                    let index = OsRng.gen_range(1..num_possible_networks);

                    possible_networks[index]
                }
                Network::Testnet | Network::Signet | Network::Regtest => Network::Bitcoin,
                other => unreachable!("this variant needs to be handled: {}", other),
            };

            assert!(
                BitcoinAddress::parse(&address_str, invalid_network)
                    .is_err_and(|e| matches!(e, ParseError::InvalidAddress(_))),
                "should error with ParseError::InvalidAddress if parse is passed an invalid address/network pair: {address_str}, {invalid_network}"
            );
        }
    }

    #[test]
    fn json_serialization_of_bitcoin_address_works() {
        // this is a random address
        // TODO: implement `Arbitrary` on `BitcoinAddress` and remove this hardcoded value
        let mainnet_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;

        let bitcoin_addr = BitcoinAddress::parse(mainnet_addr, network)
            .expect("address should be valid for the network");

        let serialized_bitcoin_addr =
            serde_json::to_string(&bitcoin_addr).expect("serialization should work");
        let deserialized_bitcoind_addr: BitcoinAddress =
            serde_json::from_str(&serialized_bitcoin_addr).expect("deserialization should work");

        assert_eq!(
            bitcoin_addr, deserialized_bitcoind_addr,
            "original and serialized addresses must be the same"
        );
    }

    #[test]
    fn borsh_serialization_of_bitcoin_address_works() {
        let mainnet_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;
        let original_addr: BitcoinAddress =
            BitcoinAddress::parse(mainnet_addr, network).expect("should be a valid address");

        let mut serialized_addr: Vec<u8> = vec![];
        original_addr
            .serialize(&mut serialized_addr)
            .expect("borsh serialization of bitcoin address must work");

        let deserialized = BitcoinAddress::try_from_slice(&serialized_addr);
        assert!(
            deserialized.is_ok(),
            "deserialization of bitcoin address should work but got: {:?}",
            deserialized.unwrap_err()
        );

        assert_eq!(
            deserialized.unwrap(),
            original_addr,
            "original address and deserialized address must be the same",
        );
    }

    #[test]
    fn test_borsh_serialization_of_multiple_addresses() {
        // Sample Bitcoin addresses
        let addresses = [
            "1BoatSLRHtKNngkdXEeobR76b53LETtpyT",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
            "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc",
        ];

        let network = Network::Bitcoin;

        // Convert strings to BitcoinAddress instances
        let bitcoin_addresses: Vec<BitcoinAddress> = addresses
            .iter()
            .map(|s| {
                BitcoinAddress::parse(s, network)
                    .unwrap_or_else(|_e| panic!("random address {s} should be valid on: {network}"))
            })
            .collect();

        // Serialize the vector of BitcoinAddress instances
        let mut serialized = Vec::new();
        bitcoin_addresses
            .serialize(&mut serialized)
            .expect("serialization should work");

        // Attempt to deserialize back into a vector of BitcoinAddress instances
        let deserialized: Vec<BitcoinAddress> =
            Vec::try_from_slice(&serialized).expect("Deserialization failed");

        // Check that the deserialized addresses match the original
        assert_eq!(bitcoin_addresses, deserialized);
    }

    #[test]
    fn test_borsh_serialization_of_address_in_struct() {
        #[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
        struct Test {
            address: BitcoinAddress,
            other: u32,
        }

        let sample_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;
        let original = Test {
            other: 1,
            address: BitcoinAddress::parse(sample_addr, network).expect("should be valid address"),
        };

        let mut serialized = vec![];
        original
            .serialize(&mut serialized)
            .expect("should be able to serialize");

        let deserialized: Test = Test::try_from_slice(&serialized).expect("should deserialize");

        assert_eq!(
            deserialized, original,
            "deserialized and original structs with address should be the same"
        );
    }

    #[test]
    fn bitcoin_addr_to_taproot_pubkey_conversion_works() {
        let network = Network::Bitcoin;
        let (address, _) = get_taproot_address(network);

        let taproot_pubkey = XOnlyPk::from_address(&address);

        assert!(
            taproot_pubkey.is_ok(),
            "conversion from address to taproot pubkey failed"
        );

        let taproot_pubkey = taproot_pubkey.unwrap();
        let bitcoin_address = taproot_pubkey.to_p2tr_address(network);

        assert!(
            bitcoin_address.is_ok(),
            "conversion from taproot pubkey to address failed"
        );

        let bitcoin_address = bitcoin_address.unwrap();
        let address_str = bitcoin_address.to_string();

        let new_taproot_pubkey = XOnlyPk::from_address(
            &BitcoinAddress::parse(&address_str, network).expect("should be a valid address"),
        );

        assert_eq!(
            bitcoin_address,
            *address.address(),
            "converted and original addresses must be the same"
        );

        assert_eq!(
            taproot_pubkey,
            new_taproot_pubkey.unwrap(),
            "converted and original taproot pubkeys must be the same"
        );
    }

    #[test]
    #[should_panic(expected = "number of sats greater than u64::MAX")]
    fn bitcoinamount_should_handle_sats_exceeding_u64_max() {
        let bitcoins: u64 = u64::MAX / BitcoinAmount::SATS_FACTOR + 1;

        BitcoinAmount::from_int_btc(bitcoins);
    }

    fn get_taproot_address(network: Network) -> (BitcoinAddress, Option<TapNodeHash>) {
        let internal_pubkey = get_random_pubkey_from_slice(&[0x12; 32]);

        let pk1 = get_random_pubkey_from_slice(&[0x02; 32]);

        let mut script1 = ScriptBuf::new();
        script1.push_slice(pk1.serialize());
        script1.push_opcode(OP_CHECKSIG);

        let pk2 = get_random_pubkey_from_slice(&[0x05; 32]);

        let mut script2 = ScriptBuf::new();
        script2.push_slice(pk2.serialize());
        script2.push_opcode(OP_CHECKSIG);

        let taproot_builder = TaprootBuilder::new()
            .add_leaf(1, script1)
            .unwrap()
            .add_leaf(1, script2)
            .unwrap();

        let tree_info = taproot_builder
            .finalize(SECP256K1, internal_pubkey)
            .unwrap();
        let merkle_root = tree_info.merkle_root();

        let taproot_address = Address::p2tr(SECP256K1, internal_pubkey, merkle_root, network);

        (
            BitcoinAddress::parse(&taproot_address.to_string(), network).unwrap(),
            merkle_root,
        )
    }

    #[test]
    fn test_bitcoinpsbt_serialize_deserialize() {
        // Create an arbitrary PSBT
        let random_data = &[0u8; 1024];
        let mut unstructured = Unstructured::new(&random_data[..]);
        let bitcoin_psbt: BitcoinPsbt = BitcoinPsbt::arbitrary(&mut unstructured).unwrap();

        // Serialize the struct
        let mut serialized = vec![];
        bitcoin_psbt
            .serialize(&mut serialized)
            .expect("Serialization failed");

        // Deserialize the struct
        let deserialized: BitcoinPsbt =
            BitcoinPsbt::deserialize(&mut &serialized[..]).expect("Deserialization failed");

        // Ensure the deserialized PSBT matches the original
        assert_eq!(bitcoin_psbt.0, deserialized.0);
    }

    #[test]
    fn test_borsh_serialize_deserialize_keypath() {
        let original = TaprootSpendPath::Key;

        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        let mut cursor = Cursor::new(serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        match deserialized {
            TaprootSpendPath::Key => (),
            _ => panic!("Deserialized variant does not match original"),
        }
    }

    #[test]
    fn test_borsh_serialize_deserialize_scriptpath() {
        // Create a sample ScriptBuf
        let script_bytes = vec![0x51, 0x21, 0xFF]; // Example script
        let script_buf = ScriptBuf::from(script_bytes.clone());

        // Create a sample ControlBlock
        let leaf_version = LeafVersion::TapScript;
        let output_key_parity = Parity::Even;

        // Generate a random internal key
        let secret_key = SecretKey::new(&mut OsRng);
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        // Create dummy TapNodeHash entries
        let tapnode_hashes = [TapNodeHash::from_byte_array([0u8; 32]); 10];

        let merkle_branch = TaprootMerkleBranch::from(tapnode_hashes);

        let control_block = ControlBlock {
            leaf_version,
            output_key_parity,
            internal_key,
            merkle_branch,
        };

        let original = TaprootSpendPath::Script {
            script_buf: script_buf.clone(),
            control_block: control_block.clone(),
        };

        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        let mut cursor = Cursor::new(serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        match deserialized {
            TaprootSpendPath::Script {
                script_buf: deserialized_script_buf,
                control_block: deserialized_control_block,
            } => {
                assert_eq!(script_buf, deserialized_script_buf, "ScriptBuf mismatch");

                // Compare ControlBlock fields
                assert_eq!(
                    control_block.leaf_version, deserialized_control_block.leaf_version,
                    "LeafVersion mismatch"
                );
                assert_eq!(
                    control_block.output_key_parity, deserialized_control_block.output_key_parity,
                    "OutputKeyParity mismatch"
                );
                assert_eq!(
                    control_block.internal_key, deserialized_control_block.internal_key,
                    "InternalKey mismatch"
                );
                assert_eq!(
                    control_block.merkle_branch, deserialized_control_block.merkle_branch,
                    "MerkleBranch mismatch"
                );
            }
            _ => panic!("Deserialized variant does not match original"),
        }
    }

    #[test]
    fn test_arbitrary_borsh_roundtrip() {
        // Generate arbitrary TaprootSpendInfo
        let data = vec![0u8; 1024];
        let mut u = Unstructured::new(&data);

        let original = TaprootSpendPath::arbitrary(&mut u).expect("Arbitrary generation failed");

        // Serialize
        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        // Deserialize
        let mut cursor = Cursor::new(&serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        // Assert equality by serializing both and comparing bytes
        let mut original_serialized = vec![];
        BorshSerialize::serialize(&original, &mut original_serialized)
            .expect("borsh serialization");

        let mut deserialized_serialized = vec![];
        BorshSerialize::serialize(&deserialized, &mut deserialized_serialized)
            .expect("borsh serialization of deserialized");

        assert_eq!(
            original_serialized, deserialized_serialized,
            "Original and deserialized serialized data do not match"
        );
    }

    #[test]
    fn test_bitcointxout_serialize_deserialize() {
        // Create a dummy TxOut with a simple script
        let script = Builder::new()
            .push_opcode(bitcoin::blockdata::opcodes::all::OP_CHECKSIG)
            .into_script();
        let tx_out = TxOut {
            value: Amount::from_sat(1000),
            script_pubkey: script,
        };

        let bitcoin_tx_out = BitcoinTxOut(tx_out);

        // Serialize the BitcoinTxOut struct
        let mut serialized = vec![];
        bitcoin_tx_out
            .serialize(&mut serialized)
            .expect("Serialization failed");

        // Deserialize the BitcoinTxOut struct
        let deserialized: BitcoinTxOut =
            BitcoinTxOut::deserialize(&mut &serialized[..]).expect("Deserialization failed");

        // Ensure the deserialized BitcoinTxOut matches the original
        assert_eq!(bitcoin_tx_out.0.value, deserialized.0.value);
        assert_eq!(bitcoin_tx_out.0.script_pubkey, deserialized.0.script_pubkey);
    }

    fn get_random_pubkey_from_slice(buf: &[u8]) -> XOnlyPublicKey {
        let sk = SecretKey::from_slice(buf).unwrap();
        let keypair = Keypair::from_secret_key(SECP256K1, &sk);
        let (pk, _) = XOnlyPublicKey::from_keypair(&keypair);

        pk
    }

    #[test]
    fn test_bitcoin_txid_serialize_deserialize() {
        let mut generator = ArbitraryGenerator::new();
        let txid: BitcoinTxid = generator.generate();

        let serialized_txid =
            borsh::to_vec::<BitcoinTxid>(&txid).expect("should be able to serialize BitcoinTxid");
        let deserialized_txid = borsh::from_slice::<BitcoinTxid>(&serialized_txid)
            .expect("should be able to deserialize BitcoinTxid");

        assert_eq!(
            deserialized_txid, txid,
            "original and deserialized txid must be the same"
        );
    }

    #[test]
    fn test_xonly_pk_to_descriptor() {
        let xonly_pk = XOnlyPk::new(Buf32::from([2u8; 32])).unwrap();
        let descriptor = xonly_pk.to_descriptor().unwrap();
        assert_eq!(descriptor.type_tag(), DescriptorType::P2tr);

        let payload = descriptor.payload();
        assert_eq!(payload.len(), 32);
        assert_eq!(payload, xonly_pk.0.as_bytes());
    }

    #[test]
    fn test_bitcoin_scriptbuf_serialize_deserialize() {
        let mut generator = ArbitraryGenerator::new();
        let scriptbuf: BitcoinScriptBuf = generator.generate();

        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );

        // Test with an empty script
        let scriptbuf: BitcoinScriptBuf = BitcoinScriptBuf(ScriptBuf::new());
        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );

        // Test with a more complex script.
        let script: ScriptBuf = ScriptBuf::from_bytes(vec![0x51, 0x21, 0xFF]); // Example script

        let scriptbuf: BitcoinScriptBuf = BitcoinScriptBuf(script);

        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );
    }
}
