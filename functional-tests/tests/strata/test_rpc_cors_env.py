"""Test that strata JSON-RPC CORS can be made permissive by env var."""

import logging

import flexitest
import requests

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)

PERMISSIVE_CORS_ENV = {"STRATA_RPC_PERMISSIVE_CORS": "1"}
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


def _assert_permissive_preflight(response):
    assert response.status_code in (200, 204), (
        f"preflight expected 200/204, got {response.status_code}"
    )
    acao = response.headers.get("access-control-allow-origin")
    assert acao in ("*", ORIGIN), f"preflight ACAO={acao!r}"
    acam = (response.headers.get("access-control-allow-methods") or "").upper()
    assert acam == "*" or "POST" in acam, f"preflight ACAM={acam!r}"
    acah = response.headers.get("access-control-allow-headers", "")
    assert acah, "preflight missing ACAH"
    logger.info("preflight ok: ACAO=%s ACAM=%s ACAH=%s", acao, acam, acah)


def _assert_protocol_version_response(response):
    assert response.status_code == 200, f"POST status {response.status_code}: {response.text!r}"
    payload = response.json()
    assert payload.get("result") == 1, f"unexpected payload: {payload!r}"
    return payload


@flexitest.register
class TestRpcCorsEnv(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, strata_env=PERMISSIVE_CORS_ENV))

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        strata.wait_for_rpc_ready(timeout=10)
        rpc_url = strata.props["rpc_url"]

        _assert_permissive_preflight(_request_preflight(rpc_url))
        res = _request_protocol_version(rpc_url)
        payload = _assert_protocol_version_response(res)
        post_acao = res.headers.get("access-control-allow-origin")
        assert post_acao in ("*", ORIGIN), f"POST ACAO={post_acao!r}"
        logger.info("cross-origin POST ok: ACAO=%s, version=%s", post_acao, payload["result"])

        return True
