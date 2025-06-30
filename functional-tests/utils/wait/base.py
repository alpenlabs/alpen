import logging
from dataclasses import dataclass
from typing import Generic, TypeVar

from utils.utils import wait_until, wait_until_with_value

T = TypeVar("T")


@dataclass
class BaseWaiter(Generic[T]):
    inner: T
    logger: logging.Logger
    timeout: int = 10
    interval: float = 0.5

    def _wait_until(self, *args, timeout=None, step=None, **kwargs):
        return wait_until(
            *args,
            timeout=timeout or self.timeout,
            step=step or self.interval,
            **kwargs,
        )

    def _wait_until_with_value(self, *args, timeout=None, step=None, **kwargs):
        return wait_until_with_value(
            *args,
            timeout=timeout or self.timeout,
            step=step or self.interval,
            **kwargs,
        )
