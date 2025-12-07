"""
Simple JSON-RPC client.
"""

import logging
from typing import Any, Callable, Optional

import requests


class RpcError(Exception):
    """Raised when an RPC call returns an error."""

    def __init__(self, error: dict):
        self.code = error.get("code")
        self.message = error.get("message")
        self.data = error.get("data")
        super().__init__(f"RPC Error {self.code}: {self.message}")


class JsonRpcClient:
    """
    JSON-RPC 2.0 client.

    Supports attribute-style method calls:
        rpc.strata_protocolVersion()
        rpc.eth_getBalance("0x123...", "latest")

    Usage:
        rpc = JsonRpcClient("http://localhost:9944")
        version = rpc.strata_protocolVersion()
    """

    def __init__(self, url: str, name: Optional[str] = None, timeout: int = 30):
        self.url = url
        self.name = name or url
        self.timeout = timeout
        self.id_counter = 0
        self.logger = logging.getLogger(f"rpc.{self.name}")
        self.pre_call_hook: Callable[[str], None] = lambda _: None

    def set_pre_call_hook(self, hook=Callable[[str], None]):
        self.pre_call_hook = hook

    def __getattr__(self, method: str):
        """
        Allow method calls as attributes.
        rpc.strata_protocolVersion() -> calls "strata_protocolVersion" method
        """

        def rpc_call(*params):
            return self._call(method, params)

        return rpc_call

    def _call(self, method: str, params: tuple) -> Any:
        """
        Make a JSON-RPC call.

        Args:
            method: RPC method name
            params: Method parameters

        Returns:
            Result from RPC call

        Raises:
            RpcError: If the RPC returns an error
            requests.RequestException: If the HTTP request fails
        """
        self.id_counter += 1

        payload = {
            "jsonrpc": "2.0",
            "method": method,
            "params": list(params),
            "id": self.id_counter,
        }

        self.logger.debug(f"RPC call: {method}({params})")

        try:
            resp = requests.post(
                self.url,
                json=payload,
                timeout=self.timeout,
            )
            resp.raise_for_status()
        except requests.RequestException as e:
            self.logger.warning(f"RPC request failed: {e}")
            raise

        try:
            result = resp.json()
        except ValueError as e:
            self.logger.warning(f"Invalid JSON response: {resp.text}")
            raise RpcError({"code": -1, "message": f"Invalid JSON: {e}"})

        if "error" in result:
            self.logger.warning(f"RPC error: {result['error']}")
            raise RpcError(result["error"])

        return result.get("result")

    def call(self, method: str, *params) -> Any:
        """
        Explicit call method (alternative to attribute style).

        Usage:
            rpc.call("strata_protocolVersion")
            rpc.call("eth_getBalance", "0x123...", "latest")
        """
        return self._call(method, params)
