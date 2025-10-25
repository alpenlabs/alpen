import flexitest
from web3 import Web3

from envs import testenv
from utils.constants import PRECOMPILE_SCHNORR_ADDRESS
from utils.precompile import (
    get_schnorr_precompile_input,
    get_test_schnnor_secret_key,
    make_precompile_call,
)


@flexitest.register
class SchnorrPrecompileTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        """
        Schnorr Precompile is available at address
        `{PRECOMPILE_SCHNORR_ADDRESS}`

        The format required is concatenation of
        `public_key` , `message_hash` and `schnorr signature` in order

        This test checks for the valid and invalid input for this precompile
        """
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        secret_key = get_test_schnnor_secret_key()
        msg = "AlpenStrata"
        precompile_input = get_schnorr_precompile_input(secret_key, msg)
        _txid, data = make_precompile_call(web3, PRECOMPILE_SCHNORR_ADDRESS, precompile_input)
        assert data == "0x01", f"Schnorr verification failed: expected '0x01', got '{data}'."

        another_message = "MakaluStrata"
        another_precompile_input = get_schnorr_precompile_input(secret_key, another_message)

        # Precompile input: Public Key (64) || Message Hash (64) || Signature (128)
        modified_precompile_input = another_precompile_input[:-128] + precompile_input[-128:]
        _txid, data = make_precompile_call(
            web3, PRECOMPILE_SCHNORR_ADDRESS, modified_precompile_input
        )
        assert data == "0x00", f"Schnorr verification failed: expected '0x00', got '{data}'."

        return True
