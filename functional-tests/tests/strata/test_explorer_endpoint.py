"""Test the embedded OL explorer HTML endpoint.

Verifies that the strata RPC port serves both:
  - `GET /explorer`  -> embedded OL explorer HTML (STR-3098)
  - `POST /`         -> existing JSON-RPC (unchanged)

Smokes the tower-layer composition wired via `set_http_middleware`.
"""

import logging

import flexitest
import requests

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestExplorerEndpoint(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        strata.wait_for_rpc_ready(timeout=10)
        rpc_url = strata.props["rpc_url"]

        # 1. GET /explorer returns the embedded HTML.
        res = requests.get(f"{rpc_url}/explorer", timeout=5)
        assert res.status_code == 200, f"expected 200, got {res.status_code}: {res.text!r}"
        ctype = res.headers.get("content-type", "")
        assert ctype.startswith("text/html"), f"content-type={ctype!r}"
        body = res.text
        # Markers from bin/strata/static/ol-explorer.html.
        assert "<title>OL Explorer</title>" in body, "title marker missing"
        assert "strata_getChainStatus" in body, "rpc method marker missing"
        assert "STR-3098" in body, "ticket marker missing"
        logger.info(f"GET /explorer: {len(body)} bytes of html, content-type={ctype}")

        # 2. JSON-RPC on the same port still works after the layer is in place.
        rpc = strata.create_rpc()
        version = rpc.strata_protocolVersion()
        assert version == 1, f"expected protocol version 1, got {version}"
        logger.info(f"POST /: jsonrpsee still serves RPC, protocolVersion={version}")

        # 3. GET on a non-explorer path is delegated to jsonrpsee, which
        #    only handles POST. Should NOT short-circuit to 200 (that would
        #    mean the layer matched the wrong path).
        res_other = requests.get(f"{rpc_url}/", timeout=5)
        assert res_other.status_code != 200, (
            f"GET / should not be served by the explorer layer; got {res_other.status_code}"
        )
        logger.info(f"GET /: status {res_other.status_code} (delegated, as expected)")

        return True
