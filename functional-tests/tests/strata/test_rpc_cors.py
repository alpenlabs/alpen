"""Test that strata JSON-RPC CORS is non-permissive by default."""

import logging

import flexitest
import requests

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)

ORIGIN = "https://example.com"
PROTOCOL_VERSION_BODY = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "strata_protocolVersion",
    "params": [],
}


def _request_preflight(rpc_url: str):
    return requests.options(
        rpc_url,
        headers={
            "Origin": ORIGIN,
            "Access-Control-Request-Method": "POST",
            "Access-Control-Request-Headers": "Content-Type",
        },
        timeout=5,
    )


def _request_protocol_version(rpc_url: str):
    return requests.post(
        rpc_url,
        json=PROTOCOL_VERSION_BODY,
        headers={"Origin": ORIGIN, "Content-Type": "application/json"},
        timeout=5,
    )


def _assert_no_cors(response, label: str):
    acao = response.headers.get("access-control-allow-origin")
    assert acao is None, f"{label} ACAO={acao!r}"


def _assert_protocol_version_response(response):
    assert response.status_code == 200, f"POST status {response.status_code}: {response.text!r}"
    payload = response.json()
    assert payload.get("result") == 1, f"unexpected payload: {payload!r}"
    return payload


@flexitest.register
class TestRpcCors(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        strata.wait_for_rpc_ready(timeout=10)
        rpc_url = strata.props["rpc_url"]

        _assert_no_cors(_request_preflight(rpc_url), "preflight")
        res = _request_protocol_version(rpc_url)
        payload = _assert_protocol_version_response(res)
        _assert_no_cors(res, "POST")
        logger.info("cross-origin POST default closed: version=%s", payload["result"])

        return True
