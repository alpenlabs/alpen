use serde::{Deserialize, Serialize};

pub use strata_identifiers::utils::get_cohashes;

use crate::prelude::Buf32;

/// Temporary schnorr keypair.
// FIXME why temporary?
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SchnorrKeypair {
    /// Secret key.
    pub sk: Buf32,

    /// Public key.
    pub pk: Buf32,
}

/// Get the temporary schnorr keypairs for testing purpose.
///
/// These are generated randomly and added here just for functional tests till we don't have proper
/// genesis configuration plus operator  addition mechanism ready
// FIXME remove
pub fn get_test_schnorr_keys() -> [SchnorrKeypair; 2] {
    let sk1 = Buf32::from([
        155, 178, 84, 107, 54, 0, 197, 195, 174, 240, 129, 191, 24, 173, 144, 52, 153, 57, 41, 184,
        222, 115, 62, 245, 106, 42, 26, 164, 241, 93, 63, 148,
    ]);

    let sk2 = Buf32::from([
        1, 192, 58, 188, 113, 238, 155, 119, 2, 231, 5, 226, 190, 131, 111, 184, 17, 104, 35, 133,
        112, 56, 145, 93, 55, 28, 70, 211, 190, 189, 33, 76,
    ]);

    let pk1 = Buf32::from([
        200, 254, 220, 180, 229, 125, 231, 84, 201, 194, 33, 54, 218, 238, 223, 231, 31, 17, 65, 8,
        94, 1, 2, 140, 184, 91, 193, 237, 28, 80, 34, 141,
    ]);

    let pk2 = Buf32::from([
        0xfa, 0x78, 0x77, 0x2d, 0x6a, 0x9a, 0xb0, 0x1a, 0x61, 0x0a, 0xb8, 0xf2, 0xfd, 0xb9, 0x01,
        0xba, 0xf3, 0x0a, 0xb2, 0x09, 0x3e, 0x53, 0xff, 0xc3, 0x1c, 0xc2, 0x81, 0xee, 0x07, 0x07,
        0x9f, 0x92,
    ]);

    [
        SchnorrKeypair { sk: sk1, pk: pk1 },
        SchnorrKeypair { sk: sk2, pk: pk2 },
    ]
}

#[cfg(test)]
mod tests {
    use bitcoin::{consensus::deserialize, Wtxid};

    use super::*;

    fn get_test_wtxids() -> Vec<Wtxid> {
        vec![
            deserialize(&[1; 32]).unwrap(),
            deserialize(&[2; 32]).unwrap(),
            deserialize(&[3; 32]).unwrap(),
            deserialize(&[4; 32]).unwrap(),
            deserialize(&[5; 32]).unwrap(),
            deserialize(&[6; 32]).unwrap(),
            deserialize(&[7; 32]).unwrap(),
        ]
    }

    #[test]
    fn test_get_cohashes_from_wtxids_idx_2() {
        let txids: Vec<Wtxid> = get_test_wtxids();
        let index = 2;

        let (proof, root) = get_cohashes(&txids, index);
        // Validate the proof length
        assert_eq!(proof.len(), 3);

        // Validate the proof contents
        assert_eq!(proof[0].0, [4; 32]);
        assert_eq!(
            proof[1].0,
            [
                57, 206, 32, 190, 222, 130, 201, 107, 137, 8, 190, 196, 161, 87, 176, 156, 84, 155,
                61, 185, 11, 155, 71, 75, 218, 154, 233, 185, 3, 3, 16, 180
            ]
        );
        assert_eq!(
            proof[2].0,
            [
                182, 31, 195, 174, 213, 89, 251, 184, 232, 133, 217, 123, 109, 127, 232, 151, 21,
                83, 204, 182, 115, 231, 30, 116, 89, 113, 163, 62, 104, 190, 1, 213
            ]
        );

        // Validate the root hash
        let expected_root = [
            92, 218, 49, 127, 159, 148, 231, 132, 215, 129, 27, 155, 152, 132, 243, 8, 47, 11, 170,
            252, 138, 147, 167, 219, 111, 149, 245, 126, 165, 46, 146, 105,
        ];
        assert_eq!(root.0, expected_root);
    }

    #[test]
    fn test_get_cohashes_from_wtxids_idx_5() {
        let txids: Vec<Wtxid> = get_test_wtxids();

        let index = 5;

        let (proof, root) = get_cohashes(&txids, index);

        // Validate the proof length
        assert_eq!(proof.len(), 3);

        // Validate the proof contents
        assert_eq!(proof[0].0, [5; 32]);
        assert_eq!(
            proof[1].0,
            [
                166, 91, 23, 162, 124, 131, 204, 95, 164, 84, 106, 176, 191, 145, 187, 217, 223,
                227, 39, 192, 18, 246, 37, 176, 214, 240, 109, 242, 54, 116, 52, 57
            ]
        );
        assert_eq!(
            proof[2].0,
            [
                8, 90, 171, 174, 249, 134, 104, 112, 27, 135, 201, 161, 152, 107, 223, 17, 103, 38,
                169, 148, 152, 2, 50, 107, 105, 137, 86, 151, 212, 232, 200, 18
            ]
        );

        // Validate the root hash
        let expected_root = [
            92, 218, 49, 127, 159, 148, 231, 132, 215, 129, 27, 155, 152, 132, 243, 8, 47, 11, 170,
            252, 138, 147, 167, 219, 111, 149, 245, 126, 165, 46, 146, 105,
        ];
        assert_eq!(root.0, expected_root);
    }
}
