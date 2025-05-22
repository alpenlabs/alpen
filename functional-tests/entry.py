#!/usr/bin/env python3
from gevent import monkey

# This is important for locust to work with flexitest.
# Because of this line, ruff linter is disabled for the whole file :(
# Currently, it's not possible to disable ruff for the block of code.
monkey.patch_all()

import argparse
import os
import sys

import flexitest

from envs import net_settings, testenv
from factory import factory
from utils import *
from utils.constants import *
from load.cfg import RethLoadConfigBuilder
from load.reth import BasicRethBlockJob, BasicRethTxJob

TEST_DIR: str = "tests"

# Initialize the parser with arguments.
parser = argparse.ArgumentParser(prog="entry.py")
parser.add_argument("-g", "--groups", nargs="*", help="Define the test groups to execute")
parser.add_argument("-t", "--tests", nargs="*", help="Define individual tests to execute")


def disabled_tests() -> list[str]:
    """
    Helper to disable some tests.
    Useful during debugging or when the test becomes flaky.
    """
    return frozenset(["basic_load"])

def filter_tests(parsed_args, modules):
    """
    Filters test modules against parsed args supplied from the command line.
    """

    arg_groups = frozenset(parsed_args.groups or [])
    # Extract filenames from the tests paths.
    arg_tests = frozenset(
        [os.path.split(t)[1].removesuffix(".py") for t in parsed_args.tests or []]
    )

    filtered = dict()
    disabled = disabled_tests()
    for test, path in modules.items():
        # Drop the prefix of the path before TEST_DIR
        test_path_parts = os.path.normpath(path).split(os.path.sep)
        # idx should never be None because TEST_DIR should be in the path.
        idx = next((i for i, part in enumerate(test_path_parts) if part == TEST_DIR), None)
        test_path_parts = test_path_parts[idx + 1 :]
        # The "groups" the current test belongs to.
        test_groups = frozenset(test_path_parts[:-1])

        # Filtering logic:
        # - check if the test is currently disabled
        # - if groups or tests were specified (non-empty) as args, then check for exclusion.
        take = test not in disabled
        if arg_groups and not (arg_groups & test_groups):
            take = False
        if arg_tests and test not in arg_tests:
            take = False

        if take:
            filtered[test] = path

    return filtered

def main(argv):
    """
    The main entrypoint for running functional tests.
    """

    parsed_args = parser.parse_args(argv[1:])

    root_dir = os.path.dirname(os.path.abspath(__file__))
    test_dir = os.path.join(root_dir, TEST_DIR)
    modules = filter_tests(parsed_args, flexitest.runtime.scan_dir_for_modules(test_dir))
    tests = flexitest.runtime.load_candidate_modules(modules)

    btc_fac = factory.BitcoinFactory([12300 + i for i in range(100)])
    seq_fac = factory.StrataFactory([12400 + i for i in range(100)])
    fullnode_fac = factory.FullNodeFactory([12500 + i for i in range(100)])
    ckpt_sync_fullnode_fac = factory.FullNodeFactory([12800 + i for i in range(100)], ["--checkpoint-sync"])
    reth_fac = factory.RethFactory([12600 + i for i in range(100 * 3)])
    prover_client_fac = factory.ProverClientFactory([12900 + i for i in range(100 * 3)])
    load_gen_fac = factory.LoadGeneratorFactory([13300 + i for i in range(100)])
    seq_signer_fac = factory.StrataSequencerFactory()

    factories = {
        "bitcoin": btc_fac,
        "sequencer": seq_fac,
        "sequencer_signer": seq_signer_fac,
        "fullnode": fullnode_fac,
        "fullnode_ckpt": ckpt_sync_fullnode_fac,
        "reth": reth_fac,
        "prover_client": prover_client_fac,
        "load_generator": load_gen_fac,
    }

    reth_load_env = testenv.LoadEnvConfig(110)
    reth_load_env.with_load_builder(
        RethLoadConfigBuilder().with_jobs([BasicRethBlockJob, BasicRethTxJob]).with_rate(30)
    )

    global_envs = {
        # Basic env is the default env for all tests.
        "basic": testenv.BasicEnvConfig(110),
        # Operator lag is a test that checks if the bridge can handle operator lag.
        # It is also useful for testing the reclaim path.
        "operator_lag": testenv.BasicEnvConfig(110, message_interval=10 * 60 * 1_000),
        # Devnet production env
        "devnet": testenv.BasicEnvConfig(110, custom_chain="devnet"),
        "hub1": testenv.HubNetworkEnvConfig(
            110
        ),  # TODO: Need to generate at least horizon blocks, based on params
        "prover": testenv.BasicEnvConfig(110, rollup_settings=RollupParamsSettings.new_default().strict_mode()),
        "load_reth": reth_load_env,
        # separate env for running crash_* tests
        "crash": testenv.BasicEnvConfig(110),
    }

    setup_root_logger()
    datadir_root = flexitest.create_datadir_in_workspace(os.path.join(root_dir, DD_ROOT))
    rt = testenv.StrataTestRuntime(global_envs, datadir_root, factories)
    rt.prepare_registered_tests()

    results = rt.run_tests(tests)
    rt.save_json_file("results.json", results)
    flexitest.dump_results(results)
    # TODO(load): dump load test stats into separate file.

    flexitest.fail_on_error(results)

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
