from abc import ABC
from dataclasses import dataclass
import logging
from typing import TypeVar

from utils.utils import wait_until, wait_until_with_value

T = TypeVar("T")

@dataclass
class BaseWaiter[T]:
    inner: T
    logger: logging.Logger
    timeout: int = 10
    interval: float = 0.5

    wait_until = staticmethod(wait_until)
    wait_until_with_value = staticmethod(wait_until_with_value)
