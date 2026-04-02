from functools import wraps
from typing import Callable, TypeVar

from bitcoinlib.services.bitcoind import BitcoindClient


T = TypeVar("T")


def with_mining(btcrpc: BitcoindClient, f: Callable[..., T]) -> Callable[..., T]:
    """
    Augments a function call with bitcoin block generation.
    Mines one block before invoking f, using a fixed address
    generated at decoration time.
    """
    mine_addr = btcrpc.proxy.getnewaddress()

    @wraps(f)
    def wrapped(*args, **kwargs):
        btcrpc.proxy.generatetoaddress(1, mine_addr)
        return f(*args, **kwargs)

    return wrapped
