"""Keep-alive test functionality for debugging environments."""

import os
import sys
import types

KEEP_ALIVE_TEST_NAME = "KeepAliveEnvTest"


def load_keepalive_test(env_name: str, test_dir: str) -> type:
    """
    Load keep-alive test with dynamic environment substitution.

    Args:
        env_name: Name of the environment to start
        test_dir: Root test directory path

    Returns:
        Test class that will start env and wait indefinitely
    """
    stub_path = os.path.join(test_dir, "keepalive_stub_test.py")

    with open(stub_path) as f:
        test_code = f.read()

    test_code = test_code.replace("{ENV}", env_name)

    module = types.ModuleType("__keepalive_dynamic_test__")
    exec(test_code, module.__dict__)
    sys.modules["__keepalive_dynamic_test__"] = module

    return getattr(module, KEEP_ALIVE_TEST_NAME)
