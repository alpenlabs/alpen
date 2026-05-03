"""Test that the strata JSON-RPC port serves permissive CORS.

The OL explorer (and any other browser-hosted tool) calls this RPC
cross-origin, so the server must respond with `Access-Control-Allow-*`
headers on both preflight (OPTIONS) and the actual JSON-RPC POST.
"""

import logging

import flexitest
import requests

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestRpcCors(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        strata.wait_for_rpc_ready(timeout=10)
        rpc_url = strata.props["rpc_url"]
        origin = "https://example.com"

        # 1. Preflight: OPTIONS with Origin + Access-Control-Request-* headers
        #    must respond with permissive ACAO/ACAM/ACAH.
        preflight = requests.options(
            rpc_url,
            headers={
                "Origin": origin,
                "Access-Control-Request-Method": "POST",
                "Access-Control-Request-Headers": "Content-Type",
            },
            timeout=5,
        )
        assert preflight.status_code in (200, 204), (
            f"preflight expected 200/204, got {preflight.status_code}"
        )
        acao = preflight.headers.get("access-control-allow-origin")
        assert acao in ("*", origin), f"preflight ACAO={acao!r}"
        acam = (preflight.headers.get("access-control-allow-methods") or "").upper()
        # `*` is a valid CORS wildcard for permissive servers; otherwise the
        # methods list must explicitly include POST.
        assert acam == "*" or "POST" in acam, f"preflight ACAM={acam!r}"
        acah = preflight.headers.get("access-control-allow-headers", "")
        assert acah, "preflight missing ACAH"
        logger.info(
            "preflight ok: ACAO=%s ACAM=%s ACAH=%s", acao, acam, acah
        )

        # 2. Actual cross-origin POST: response must echo ACAO so the
        #    browser surfaces the body to the calling page.
        body = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "strata_protocolVersion",
            "params": [],
        }
        res = requests.post(
            rpc_url,
            json=body,
            headers={"Origin": origin, "Content-Type": "application/json"},
            timeout=5,
        )
        assert res.status_code == 200, f"POST status {res.status_code}: {res.text!r}"
        post_acao = res.headers.get("access-control-allow-origin")
        assert post_acao in ("*", origin), f"POST ACAO={post_acao!r}"
        payload = res.json()
        assert payload.get("result") == 1, f"unexpected payload: {payload!r}"
        logger.info("cross-origin POST ok: ACAO=%s, version=%s", post_acao, payload["result"])

        return True
