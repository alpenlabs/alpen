import time

import flexitest

from envs import testenv


@flexitest.register
class KeepAliveEnvMockTest(testenv.StrataTestBase):
    """
    A dynamically populated mock test for the keep-alive mode.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("{ENV}")

    def main(self, _ctx: flexitest.RunContext):
        while True:
            print("running fn-tests in keep-alive mode")
            time.sleep(60)
