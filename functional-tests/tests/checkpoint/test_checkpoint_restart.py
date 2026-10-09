"""Restart preserves proven checkpoints that ASM has not accepted yet."""

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import run_dbtool_json


@flexitest.register
class TestCheckpointRestart(StrataNodeTest):
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
        signer = self.get_service(ServiceType.StrataSigner)
        rpc = strata.wait_for_rpc_ready(timeout=20)
        admin_rpc = strata.create_admin_rpc()
        btc_rpc = bitcoin.create_rpc()
        # Materialize post-genesis canonical ASM state before testing restart.
        btc_rpc.proxy.generatetoaddress(6, btc_rpc.proxy.getnewaddress())
        l1_tip = btc_rpc.proxy.getblockcount()
        strata.wait_for_asm_manifest_commitment_at(l1_tip, rpc=rpc, timeout=30)
        epoch = int(rpc.strata_getChainStatus()["latest"]["epoch"]) + 1

        # Keep Bitcoin fixed: OL completes epochs, but ASM cannot accept their
        # checkpoints. Submission succeeds only after the proven payload exists.
        # The RPC ignores the signature; the envelope authenticates publication.
        wait_until_with_value(
            lambda: admin_rpc.strata_strataadmin_completeCheckpointSignature(epoch, "00" * 64),
            lambda _: True,
            error_with=f"epoch {epoch} checkpoint was not ready for submission",
            timeout=90,
        )
        assert rpc.strata_getCheckpointInfo(epoch)["confirmation_status"]["status"] == "pending"

        signer.stop()
        strata.stop()
        datadir = strata.props["datadir"]
        checkpoint = run_dbtool_json(datadir, "get-checkpoint", str(epoch))
        proof = run_dbtool_json(datadir, "get-checkpoint-proof", str(epoch))
        tasks = run_dbtool_json(
            datadir, "get-prover-tasks-summary", "--status", "completed", "--limit", "100"
        )["entries"]
        assert "intent_index" in checkpoint, checkpoint
        assert proof["proof_len"] > 0, proof
        assert tasks, "expected a completed checkpoint proof task"

        # Leave the signer stopped so it cannot recreate a deleted signing row.
        strata.start()
        rpc = strata.wait_for_rpc_ready(timeout=30)
        assert rpc.strata_getCheckpointInfo(epoch)["confirmation_status"]["status"] == "pending"
        strata.stop()

        assert run_dbtool_json(datadir, "get-checkpoint", str(epoch)) == checkpoint
        assert run_dbtool_json(datadir, "get-checkpoint-proof", str(epoch)) == proof
        for task in tasks:
            assert run_dbtool_json(datadir, "get-prover-task", task["key_hex"]) == task
        assert btc_rpc.proxy.getblockcount() == l1_tip
        return True
