from utils.wait.base import RpcWaiter


class RethWaiter(RpcWaiter):
    def wait_until_eth_block_exceeds(self, height, message: str | None = None):
        message = message or f"Timeout: waiting for block at height {height}"
        return self._wait_until_with_value(
            lambda: int(self.rpc_client.eth_blockNumber(), 16),
            lambda value: value > height,
            error_with=message,
            timeout=self.timeout,
            step=self.interval,
        )

    def wait_until_eth_block_at_least(self, height, message: str | None = None):
        """
        Waits until eth block number reaches at least the specified height.
        """
        return self._wait_until_with_value(
            lambda: int(self.rpc_client.eth_blockNumber(), 16),
            lambda value: value >= height,
            error_with=message or f"Timeout: waiting for block height {height}",
            timeout=self.timeout,
            step=self.interval,
        )

    def get_current_block_number(self) -> int:
        """
        Get the current block number from reth RPC.
        """
        return int(self.rpc_client.eth_blockNumber(), 16)

    def wait_until_state_diff_at_blockhash(self, blockhash, timeout: None | int = None):
        return self._wait_until_with_value(
            lambda: self.rpc_client.strataee_getBlockStateDiff(blockhash),
            lambda value: value is not None,
            error_with="Finding non empty statediff for blockhash {blockhash} timed out",
            timeout=timeout or self.timeout,
        )

    def wait_until_block_witness_at_blockhash(self, blockhash, timeout: None | int = None):
        return self._wait_until_with_value(
            # TODO: parameterize True
            lambda: self.rpc_client.strataee_getBlockWitness(blockhash, True),
            lambda value: value is not None,
            error_with="Finding non empty witness for blockhash {blockhash} timed out",
            timeout=timeout or self.timeout,
        )

    def wait_until_tx_included_in_block(self, txid: str):
        def _query():
            try:
                receipt = self.rpc_client.get_transaction_receipt(txid)
                return receipt
            except Exception as e:
                return e

        result = self._wait_until_with_value(
            _query,
            lambda result: not isinstance(result, Exception),
            error_with="Transaction receipt for txid not available",
            timeout=self.timeout,
        )
        return result
