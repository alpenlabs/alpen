import logging

import flexitest
import time

from envs import testenv

WAIT_TIME = 2


@flexitest.register
class ExampleTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        time.sleep(10000)
        return
