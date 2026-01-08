use serde::{Deserialize, Serialize};

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
