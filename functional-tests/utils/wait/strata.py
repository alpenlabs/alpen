from typing import Any

from factory.seqrpc import RpcError
from utils.wait.base import RpcWaiter


class StrataWaiter(RpcWaiter):
    def wait_until_genesis(self, message: str | None = None):
        """
        Waits until we see genesis. That is to say, that `strata_syncStatus`
        returns a sensible result.
        """

        msg = message or "Timeout: waiting for genesis"

        def _check_genesis():
            try:
                # This should raise if we're before genesis.
                ss = self.rpc_client.strata_syncStatus()
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

        self._wait_until(_check_genesis, timeout=self.timeout, step=self.interval, error_with=msg)

    def wait_until_chain_epoch(
        self,
        epoch: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until the chain has finished the specified epoch index, determined by
        checking for epoch summaries.

        Returns the epoch summary.
        """
        self.logger.info(f"waiting for epoch {epoch}")

        def _query():
            status = self.rpc_client.strata_syncStatus()
            self.logger.debug(f"checked status {status}")
            commitments = self.rpc_client.strata_getEpochCommitments(epoch)
            if len(commitments) > 0:
                comm = commitments[0]
                self.logger.info(
                    f"now at epoch {epoch}, slot {comm['last_slot']}, blkid {comm['last_blkid']}"
                )
                return self.rpc_client.strata_getEpochSummary(
                    epoch,
                    comm["last_slot"],
                    comm["last_blkid"],
                )
            return None

        def _check(v):
            return v is not None

        msg = message or "Timeout: waiting for chain epoch"

        return self._wait_until_with_value(
            _query,
            _check,
            timeout=timeout,
            step=interval,
            error_with=msg,
        )

    def wait_until_next_chain_epoch(
        self, timeout: int | None = None, interval: float | None = None, message: str | None = None
    ) -> int:
        """
        Waits until the chain epoch advances by at least 1.

        Returns the new epoch number.
        """
        init_epoch = self.rpc_client.strata_syncStatus()["cur_epoch"]

        def _query():
            ss = self.rpc_client.strata_syncStatus()
            self.logger.info(f"waiting for next epoch, ss {ss}")
            return ss["cur_epoch"]

        def _check(epoch):
            return epoch > init_epoch

        error_with = message or "Timeout waiting for next epoch"

        return self._wait_until_with_value(
            _query,
            _check,
            timeout=timeout,
            step=interval,
            error_with=error_with,
        )

    def wait_until_epoch_confirmed(
        self,
        epoch: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until at least the given epoch is confirmed on L1, according to
        calling `strata_clientStatus`.
        """

        def _check():
            cs = self.rpc_client.strata_clientStatus()
            l1_height = cs["tip_l1_block"]["height"]
            conf_epoch = cs["confirmed_epoch"]
            self.logger.info(f"confirmed epoch as of {l1_height}: {conf_epoch}")
            if conf_epoch is None:
                return False
            return conf_epoch["epoch"] >= epoch

        error_with = message or f"Timeout waiting for epoch {epoch} to be confirmed"

        self._wait_until(_check, timeout=timeout, step=interval, error_with=error_with)

    def wait_until_chain_tip_exceeds(
        self, height: int, timeout: int | None = None, msg: str | None = None
    ):
        """
        Waits until strata chain tip exceeds the given height.
        """
        return self._wait_until_with_value(
            lambda: self.rpc_client.strata_syncStatus(),
            lambda stat: stat["tip_height"] > height,
            error_with=msg or "Timeout: expected number of blocks are not being created",
            timeout=timeout,
        )

    def wait_until_epoch_finalized(
        self,
        epoch: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until at least the given epoch is finalized on L1, according to
        calling `strata_clientStatus`.
        """

        def _check():
            cs = self.rpc_client.strata_clientStatus()
            l1_height = cs["tip_l1_block"]["height"]
            fin_epoch = cs["finalized_epoch"]
            self.logger.info(f"finalized epoch as of {l1_height}: {fin_epoch}")
            if fin_epoch is None:
                return False
            return fin_epoch["epoch"] >= epoch

        error_with = message or f"Timeout waiting for epoch {epoch} to be finalized"

        self._wait_until(
            _check,
            timeout=timeout,
            step=interval,
            error_with=error_with,
        )

    def wait_until_client_ready(
        self, timeout: int | None = None, interval: float | None = None, message: str | None = None
    ):
        """
        Waits until the strata client is ready to serve rpc
        """
        message = message or "Strata client did not start on time"

        self._wait_until(
            lambda: self.rpc_client.strata_protocolVersion() is not None,
            error_with=message,
            timeout=timeout,
            step=interval,
        )

    def wait_until_epoch_observed_final(
        self,
        epoch: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until at least the given epoch is observed as final on L2, according
        to calling `strata_syncStatus`.
        """

        def _check():
            ss = self.rpc_client.strata_syncStatus()
            slot = ss["tip_height"]  # TODO rename to tip_slot
            of_epoch = ss["observed_finalized_epoch"]
            self.logger.info(f"observed final epoch as of L2 slot {slot}: {of_epoch}")
            if not of_epoch:
                return False
            return of_epoch["epoch"] >= epoch

        error_with = message or f"Timeout waiting for epoch {epoch} to be observed as final"

        self._wait_until(
            _check,
            timeout=timeout,
            step=interval,
            error_with=error_with,
        )

    def wait_until_l1_observed(
        self,
        height: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until the provided L1 height has been observed by the chain.
        """

        def _check():
            ss = self.rpc_client.strata_syncStatus()
            slot = ss["tip_height"]  # TODO rename to slot
            epoch = ss["cur_epoch"]
            view_l1 = ss["safe_l1_block"]["height"]
            self.logger.info(
                f"chain now at slot {slot}, epoch {epoch}, observed L1 height is {view_l1}"
            )
            return view_l1 >= height

        error_with = message or f"Timeout waiting for L1 height {height} to be observed"

        self._wait_until(_check, timeout=timeout, step=interval, error_with=error_with)

    def wait_until_l1_height_at(
        self,
        height: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ) -> Any:
        """
        Waits until strata client's reader sees L1 block at least upto given height.

        Returns the latest L1Status.
        """
        message = message or "L1 reader did not catch up with bitcoin network"

        return self._wait_until_with_value(
            lambda: self.rpc_client.strata_l1status(),
            lambda value: value["cur_height"] >= height,
            error_with=message,
            timeout=timeout,
            step=interval,
        )

    def wait_until_recent_block_headers_at(
        self,
        height: int,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ) -> Any:
        """
        Waits until recent block headers are available at given height.
        """
        message = message or "Blocks not generated"

        return self._wait_until_with_value(
            lambda: self.rpc_client.strata_getRecentBlockHeaders(height),
            lambda value: value is not None,
            error_with=message,
            timeout=timeout,
            step=interval,
        )

    def wait_until_csm_l1_tip_observed(
        self, timeout: int | None = None, interval: float | None = None, message: str | None = None
    ):
        """
        Waits until the CSM's current L1 tip block height has been observed by the OL.
        """
        init_cs = self.rpc_client.strata_clientStatus()
        init_l1_height = init_cs["tip_l1_block"]["height"]
        self.logger.info(f"target L1 height from CSM is {init_l1_height}")
        self.wait_until_l1_observed(
            init_l1_height, timeout=timeout, interval=interval, message=message
        )

    def wait_until_cur_l1_tip_observed(
        self,
        btcrpc,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until the current L1 tip block as requested from the L1 RPC has been
        observed by the CSM.

        Returns the L1 block height.
        """
        info = btcrpc.proxy.getblockchaininfo()
        h = info["blocks"]
        self.logger.info(f"current bitcoin height is {h}")
        self.wait_until_l1_observed(h, timeout=timeout, interval=interval, message=message)
        return h

    def wait_until_latest_checkpoint_at(self, idx: int, timeout: int | None = None):
        self._wait_until(
            lambda: self.rpc_client.strata_getLatestCheckpointIndex(None) >= idx,
            timeout=timeout,
            error_with=f"Timeout: Checkpoint index did not increment to expected value({idx})",
        )

    def wait_until_asm_ready(
        self,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until ASM state is available and operational.
        This should be called after genesis before attempting to access bridge data.

        The ASM worker processes L1 blocks asynchronously to build manifests and
        subprotocol state (like BridgeV1). This method ensures the ASM state is
        initialized and ready for queries.
        """
        msg = message or "Timeout waiting for ASM state to be ready"

        def _check():
            try:
                # Try to fetch current deposits - this will fail if ASM state isn't ready
                self.rpc_client.strata_getCurrentDeposits()
                self.logger.info("ASM state is ready and operational")
                return True
            except RpcError as e:
                # ASM state not found or not ready yet
                error_str = str(e)
                if any(
                    phrase in error_str
                    for phrase in [
                        "ASM state",
                        "BridgeV1 section not found",
                        "failed to load BridgeV1 state",
                    ]
                ):
                    self.logger.debug(f"ASM not ready yet: {error_str}")
                    return False
                # Re-raise if it's a different error
                raise e

        self._wait_until(_check, timeout=timeout, step=interval, error_with=msg)

    def wait_until_bridge_operational(
        self,
        timeout: int | None = None,
        interval: float | None = None,
        message: str | None = None,
    ):
        """
        Waits until the BridgeV1 subprotocol is operational within the ASM state.

        This is a more specific check than wait_until_asm_ready, ensuring that
        not only is the ASM state available, but the BridgeV1 section specifically
        is initialized and can be queried.
        """
        msg = message or "Timeout waiting for BridgeV1 subprotocol to be operational"

        def _check():
            try:
                # Try multiple bridge-related queries to ensure full operability
                deposits = self.rpc_client.strata_getCurrentDeposits()
                withdrawals = self.rpc_client.strata_getCurrentWithdrawalAssignments()
                operators = self.rpc_client.strata_getActiveOperatorChainPubkeySet()

                self.logger.info(
                    f"Bridge operational: {len(deposits)} deposits, "
                    f"{len(withdrawals)} withdrawals, {len(operators)} operators"
                )
                return True
            except RpcError as e:
                error_str = str(e)
                # Check for ASM/Bridge-related errors
                if any(
                    phrase in error_str
                    for phrase in [
                        "ASM state",
                        "BridgeV1 section",
                        "failed to load BridgeV1",
                    ]
                ):
                    self.logger.debug(f"Bridge not operational yet: {error_str}")
                    return False
                # Re-raise if it's a different error
                raise e

        self._wait_until(_check, timeout=timeout, step=interval, error_with=msg)
