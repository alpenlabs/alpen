import logging
import time

import flexitest

from envs import testenv

WAIT_TIME = 2


@flexitest.register
class BridgeMsgTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        logging.warn("test temporarily disabled")
        return

        seq = ctx.get_service("sequencer")

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()

        # NOTE: the Bridge config should have relay_misc set to True in order
        # for this to pass since the scope of the message is Misc

        # BridgeMessage { source_id: 1,
        #                 sig: [00] * 64
        #                 scope: Misc, payload: [42] }
        raw_msg = "".join(
            [
                "01000000",
                "00" * 64,
                "01000000" + "00",
                "01000000" + "42",
            ]
        )

        seqrpc.strata_submitBridgeMsg(raw_msg)

        # FIXME: is it possible to wait for some condition here?
        time.sleep(WAIT_TIME + 2)

        # VODepositSig(10)
        scope = "00"
        self.debug(scope)

        msgs = seqrpc.strata_getBridgeMsgsByScope(scope)
        self.debug(msgs)

        # check if received blobdata and sent blobdata are same or not
        assert len(msgs) == 1, "wrong number of messages in response"
        assert msgs[0] == raw_msg, "not the message we expected"
