"""
Keep-alive test template for debugging environments.
This file is used by entry.py with dynamic environment substitution.
"""

import time

import flexitest

from common.base_test import BaseTest


@flexitest.register
class KeepAliveEnvTest(BaseTest):
    """Keep-alive test for debugging. Environment is dynamically set."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("{ENV}")

    def run(self) -> bool:
        print("\n" + "=" * 60)
        print("Keep-alive mode: Environment '{ENV}' is running")
        print("Press Ctrl+C to stop...")
        print("=" * 60 + "\n")

        try:
            while True:
                time.sleep(60)
                print("Keep-alive: environment still running...")
        except KeyboardInterrupt:
            print("\nShutting down...")
            return True
