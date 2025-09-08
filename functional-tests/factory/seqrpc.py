import json

import requests
from websockets.sync.client import connect as wsconnect


class RpcError(Exception):
    def __init__(self, code: int, msg: str, data=None):
        self.code = code
        self.msg = msg
        self.data = data

    def __str__(self) -> str:
        return f"RpcError: code {self.code} ({self.msg})"


def _make_request(method: str, req_id: int, params) -> str:
    """Assembles a request body from parts."""
    req = {"jsonrpc": "2.0", "method": method, "id": req_id, "params": params}
    return json.dumps(req)


def _handle_response(resp_str: str):
    """Takes a response body and extracts the result or raises the error."""
    resp = json.loads(resp_str)
    if "error" in resp:
        e = resp["error"]
        d = None
        if "data" in e:
            d = e["data"]
        raise RpcError(e["code"], e["message"], data=d)
    return resp["result"]


def _send_single_ws_request(url: str, request: str, max_size: int | None = None) -> str:
    with wsconnect(url, max_size=max_size) as w:
        w.send(request)
        return w.recv()


def _send_http_request(url: str, request: str) -> str:
    h = {"Content-Type": "application/json"}
    res = requests.post(url, headers=h, data=request)
    return res.text


def _dispatch_request(url: str, request: str, max_size: int | None = None) -> str:
    if url.startswith("http"):
        return _send_http_request(url, request)
    elif url.startswith("ws"):
        return _send_single_ws_request(url, request, max_size=max_size)
    else:
        raise ValueError(f"unsupported protocol in url '{url}'")


class JsonrpcClient:
    def __init__(self, url: str):
        self.url = url
        self.req_idx = 0
        # Hook that lets us add a check that runs before every call.
        self._pre_call_hook = None

    def _do_pre_call_check(self, m: str):
        """Calls the pre-call hook if set."""
        if self._pre_call_hook is not None:
            h = self._pre_call_hook
            r = h(m)
            if type(r) is bool:
                if not r:
                    raise RuntimeError(f"failed precheck on call to '{m}'")

    def _call(self, method: str, args, **kwargs):
        max_size = kwargs.get("max_size")
        self._do_pre_call_check(method)
        req = _make_request(method, self.req_idx, args)
        self.req_idx += 1
        resp = _dispatch_request(self.url, req, max_size=max_size)
        return _handle_response(resp)

    def __getattr__(self, name: str):
        def __call(*args, **kwargs):
            return self._call(name, args, **kwargs)

        return __call
