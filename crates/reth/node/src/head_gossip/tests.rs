use proptest::prelude::*;
use reth_primitives::Header;

use crate::head_gossip::protocol::HeadGossipMessage;

proptest! {
    #[test]
    fn roundtrip_head_hash(
        parent_hash in any::<[u8; 32]>(),
        state_root in any::<[u8; 32]>(),
        timestamp in any::<u64>(),
        number in any::<u64>(),
    ) {
        let header = Header {
            parent_hash: parent_hash.into(),
            state_root: state_root.into(),
            timestamp,
            number,
            ..Default::default()
        };

        let msg = HeadGossipMessage::new_head_hash(header);
        let encoded = msg.encoded();
        let decoded = HeadGossipMessage::decode_message(&mut &encoded[..]).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn roundtrip_head_hashes(
        headers_data in prop::collection::vec((any::<[u8; 32]>(), any::<u64>()), 0..10)
    ) {
        let headers: Vec<Header> = headers_data.into_iter().map(|(hash, num)| {
            Header {
                parent_hash: hash.into(),
                number: num,
                ..Default::default()
            }
        }).collect();

        let msg = HeadGossipMessage::new_head_hashes(headers);
        let encoded = msg.encoded();
        let decoded = HeadGossipMessage::decode_message(&mut &encoded[..]).unwrap();
        assert_eq!(msg, decoded);
    }
}
