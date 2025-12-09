#!/usr/bin/env python3
"""
Functional test runner.

Usage:
    ./entry.py                    # Run all tests
    ./entry.py -t test_node       # Run specific test
    ./entry.py -g bridge          # Run test group
"""

import argparse
import logging
import os
import sys

import flexitest

from common.config import ServiceType

# Import environments
from envconfigs.basic import BasicEnvConfig

# Import factories
from factories.bitcoin import BitcoinFactory
from factories.strata import StrataFactory


def setup_logging() -> None:
    """Configure root logger."""
    log_level = os.getenv("LOG_LEVEL", "INFO").upper()
    logging.basicConfig(
        level=getattr(logging, log_level, logging.INFO),
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    )


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(
        prog="entry.py",
        description="Run functional tests",
    )
    parser.add_argument(
        "-t",
        "--test",
        nargs="*",
        help="Run specific test(s)",
    )
    parser.add_argument(
        "-g",
        "--group",
        nargs="*",
        help="Run test group(s)",
    )
    return parser.parse_args(argv[1:])


def main(argv: list[str]) -> int:
    """Main entry point."""
    args = parse_args(argv)
    setup_logging()

    # Create factories
    factories: dict[ServiceType, flexitest.Factory] = {
        ServiceType.Bitcoin: BitcoinFactory(range(18443, 18543)),
        ServiceType.Strata: StrataFactory(range(19443, 19543)),
    }

    # Define global environments
    global_envs: dict[str, flexitest.EnvConfig] = {
        "basic": BasicEnvConfig(pre_generate_blocks=110),
    }

    # Set up test runtime
    root_dir = os.path.dirname(os.path.abspath(__file__))
    datadir = flexitest.create_datadir_in_workspace(os.path.join(root_dir, "_dd"))
    runtime = flexitest.TestRuntime(global_envs, datadir, factories)

    # Discover tests
    test_dir = os.path.join(root_dir, "tests")
    modules = flexitest.runtime.scan_dir_for_modules(test_dir)

    # TODO: Add test filtering based on args.test and args.group
    # For now, args.test and args.group are parsed but ignored
    if args.test or args.group:
        print(
            "Warning: Test filtering (--test, --group) is not yet implemented. Running all tests."
        )

    tests = flexitest.runtime.load_candidate_modules(modules)

    # Run tests
    runtime.prepare_registered_tests()
    results = runtime.run_tests(tests)

    # Save and display results
    runtime.save_json_file("results.json", results)
    flexitest.dump_results(results)

    # Exit with error if any test failed
    flexitest.fail_on_error(results)

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
