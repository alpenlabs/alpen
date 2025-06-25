from abc import ABC
from dataclasses import dataclass
from logging import Logger
from typing import Any

from utils.utils import wait_until, wait_until_with_value


@dataclass
class BaseWaiter(ABC):
    rpc: Any
    logger: Logger
    timeout: int = 10
    interval: float = 0.5

    def __post_init__(self):
        self.wait_until = wait_until
        self.wait_until_with_value = wait_until_with_value
