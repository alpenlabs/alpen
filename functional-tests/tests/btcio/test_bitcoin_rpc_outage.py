"""Bitcoin RPC outages defer checkpoint publication without restarting Strata."""

import time

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch


@flexitest.register
class TestBitcoinRpcOutage(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig.new_fixed_slot(4),
                ol_block_time_ms=1_000,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)
        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()
        btc_rpc.proxy.generatetoaddress(6, btc_rpc.proxy.getnewaddress())
        strata.wait_for_asm_manifest_commitment_at(btc_rpc.proxy.getblockcount(), rpc=rpc)

        # Keep L1 fixed while the checkpoint commit/reveal reaches the mempool.
        pending = set(
            wait_until_with_value(
                btc_rpc.proxy.getrawmempool,
                lambda txids: len(txids) >= 2,
                error_with="Checkpoint commit/reveal did not reach the mempool",
                timeout=90,
            )
        )
        process = strata.proc
        assert process is not None

        # The fixture creates its wallet by RPC; load it during restart before warmup ends.
        bitcoin.cmd.append("-wallet=testwallet")
        bitcoin.stop()
        try:
            # Connection refusals exhaust the client's three one-second retries.
            # Stay down for several worker polls and check liveness throughout.
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                assert strata.check_status(), "Strata exited during Bitcoin RPC outage"
                rpc.strata_protocolVersion()
                time.sleep(0.5)
        finally:
            bitcoin.start()

        bitcoin.wait_for_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        wait_until_with_value(
            btc_rpc.proxy.getrawmempool,
            lambda txids: pending.issubset(txids),
            error_with="Previously queued transactions did not recover after Bitcoin restart",
            timeout=60,
        )
        # This epoch has not completed yet, so its publication must happen after recovery.
        target_epoch = rpc.strata_getChainStatus()["latest"]["epoch"] + 1
        mine_until_finalized_epoch(bitcoin, strata, rpc, target_epoch=target_epoch, timeout=120)
        for txid in pending:
            assert btc_rpc.proxy.getrawtransaction(txid, 1)["confirmations"] > 0
        assert strata.proc is process and strata.check_status(), "Strata process changed"
        return True
