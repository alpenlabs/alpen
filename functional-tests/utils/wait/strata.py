from dataclasses import dataclass
from logging import Logger
from typing import Any

from factory.seqrpc import RpcError
from utils.utils import wait_until, wait_until_with_value


@dataclass
class StrataWaiter:
    """
    Wrapper for encapsulating and waiting strata related rpcs
    """
    strata_rpc: Any
    logger: Logger
    timeout: int = 10
    interval: float = 0.5

    def wait_for_genesis(self, message=None):
        """
        Waits until we see genesis. That is to say, that `strata_syncStatus`
        returns a sensible result.
        """

        msg = message or "Timeout: waiting for genesis"
        def _check_genesis():
            try:
                # This should raise if we're before genesis.
                ss = self.strata_rpc.strata_syncStatus()
                self.logger.info(
                    f"after genesis, tip is slot {ss['tip_height']} blkid {ss['tip_block_id']}"
                )
                return True
            except RpcError as e:
                # This is the "before genesis" error code, meaning we're still
                # before genesis
                if e.code == -32607:
                    return False
                else:
                    raise e

        wait_until(_check_genesis, timeout=self.timeout, step=self.interval, error_with=msg)
