import time
from dataclasses import dataclass
from logging import Logger
from typing import Any

from utils.utils import wait_until


@dataclass
class ProverWaiter:
    """
    Wrapper for encapsulating and waiting prover related operations
    """

    prover_rpc: Any
    logger: Logger
    timeout: int = 60
    interval: float = 2.0
    message: str = "Timeout: waiting for proof completion"

    def wait_for_proof_completion(self, task_id: str, timeout: int | None = None) -> bool:
        """
        Waits for a proof task to complete/fail within a specified timeout period.

        This function continuously polls the status of a proof task identified by `task_id` using
        the `prover_rpc` client. It checks the status every interval and waits until the
        proof task status is either "Completed" where it returns True, or "Failed" where it returns
        False. If the specified `timeout` (in seconds) is reached, it throws TimeoutError.

        Args:
            task_id: The proof task identifier
            timeout: Override timeout in seconds (default uses class timeout)

        Returns:
            bool: True if completed successfully, False if failed

        Raises:
            TimeoutError: If operation times out
        """
        timeout = timeout or self.timeout
        start_time = time.time()

        while True:
            # Fetch the proof status
            proof_status = self.prover_rpc.dev_strata_getTaskStatus(task_id)
            assert proof_status is not None
            self.logger.info(f"Got the proof status {proof_status}")

            if proof_status == "Completed":
                self.logger.info(f"Completed the proof generation for {task_id}")
                return True
            elif proof_status == "Failed":
                self.logger.info(f"Proof generation failed for {task_id}")
                return False

            elapsed_time = time.time() - start_time  # Calculate elapsed time
            if elapsed_time >= timeout:
                raise TimeoutError(f"Proof generation timed out after {timeout} seconds.")

            time.sleep(self.interval)

    def wait_for_prover_ready(self, timeout: int | None = None):
        """
        Waits until the prover client reports readiness.

        Args:
            timeout: Override timeout in seconds (default uses class timeout)
        """

        timeout = timeout or self.timeout
        wait_until(
            lambda: self.prover_rpc.dev_strata_getReport() is not None,
            error_with="Prover did not start on time",
            timeout=timeout,
            step=self.interval,
        )
