import flexitest

from envs import testenv
from envs.testenv import BasicEnvConfig
from utils import (
    ProverClientSettings,
    RollupParamsSettings,
    wait_until,
)


@flexitest.register
class ProverClientRestartTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        # A separate standalone env for this test as it involves a restart
        # and the rollup settings are non-standard.

        # Increase the proof timeout so that the checkpoint index increments only
        # after the prover client submits the corresponding checkpoint proof
        rollup_settings = RollupParamsSettings.new_default()
        rollup_settings.proof_timeout = 300

        ctx.set_env(
            BasicEnvConfig(
                pre_generate_blocks=101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=rollup_settings,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        prover_client = ctx.get_service("prover_client")
        prover_client_rpc = prover_client.create_rpc()
        sequencer = ctx.get_service("sequencer")
        sequencer_rpc = sequencer.create_rpc()

        # Wait for the Prover Manager setup
        wait_until(
            lambda: prover_client_rpc.dev_strata_getReport() is not None,
            error_with="Prover did not start on time",
        )

        # Test on with the latest checkpoint
        latest_checkpoint = sequencer_rpc.strata_getLatestCheckpointIndex()
        self.prove_latest_checkpoint(prover_client_rpc)

        self.debug("restart prover client")
        prover_client.stop()
        prover_client.start()
        prover_client_rpc = prover_client.create_rpc()

        self.debug("prover client restarted, waiting for the new checkpoint")
        wait_until(
            lambda: sequencer_rpc.strata_getLatestCheckpointIndex() == latest_checkpoint + 1,
            timeout=180,
            step=5.0,
        )

        self.prove_latest_checkpoint(prover_client_rpc)

    def prove_latest_checkpoint(self, prover_client_rpc):
        task_ids = prover_client_rpc.dev_strata_proveLatestCheckPoint()
        prover_waiter = self.create_prover_waiter(prover_client_rpc, timeout=30)
        self.debug(f"got task ids: {task_ids}")
        task_id = task_ids[0]
        self.debug(f"using task id: {task_id}")
        assert task_id is not None

        is_proof_generation_completed = prover_waiter.wait_for_proof_completion(task_id)
        assert is_proof_generation_completed
