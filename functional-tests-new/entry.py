#!/usr/bin/env python3
"""
Functional test runner.

Usage:
    ./entry.py                          # Run all tests
    ./entry.py -t test_node_version     # Run specific test
    ./entry.py -t tests/test_node_version.py  # Run specific test by path
    ./entry.py tests/test_node_version.py     # Run specific test (positional)
    ./entry.py -g bridge                # Run test group (directory-based)
    ./entry.py -e basic                 # Keep-alive mode for debugging
    ./entry.py --list                   # List all tests
"""

import argparse
import logging
import os
import sys

import flexitest
from flexitest.runtime import load_candidate_modules, scan_dir_for_modules

from common.config import ServiceType
from common.keepalive import KEEP_ALIVE_TEST_NAME, load_keepalive_test

# Import environments
from common.runtime import TestRuntimeWithLogging
from common.test_logging import TestNameFilter
from envconfigs.alpen_client import AlpenClientEnv
from envconfigs.strata import StrataEnvConfig

# Import factories
from factories.alpen_client import AlpenClientFactory
from factories.bitcoin import BitcoinFactory
from factories.strata import StrataFactory


def disabled_tests() -> frozenset[str]:
    """
    Tests to disable (e.g., flaky tests, work-in-progress).

    Returns test names without .py extension.
    Can be extended via DISABLED_TESTS env var (comma-separated).
    """
    base_disabled = frozenset(["keepalive_stub_test"])

    env_disabled = os.getenv("DISABLED_TESTS", "")
    if env_disabled:
        env_set = frozenset(t.strip() for t in env_disabled.split(",") if t.strip())
        return base_disabled | env_set

    return base_disabled


def setup_logging() -> None:
    """Configure root logger with test name filter."""
    log_level = os.getenv("LOG_LEVEL", "INFO").upper()
    logging.basicConfig(
        level=getattr(logging, log_level, logging.INFO),
        format="%(asctime)s - [%(test_name)s] - %(name)s - %(levelname)s - %(message)s",
    )
    for handler in logging.root.handlers:
        handler.addFilter(TestNameFilter())


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(
        prog="entry.py",
        description="Run functional tests",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s                          Run all tests
  %(prog)s -t test_node_version     Run specific test
  %(prog)s -t tests/test_node_version.py  Run specific test by path
  %(prog)s tests/test_node_version.py     Run specific test (positional)
  %(prog)s -t test_foo test_bar     Run multiple tests
  %(prog)s -g bridge                Run all tests in bridge/ directory
  %(prog)s -g prover bridge         Run tests in multiple groups
  %(prog)s --keep-alive basic       Start 'basic' env and keep it alive (no tests run)
  %(prog)s --list                   List all available tests
        """,
    )
    parser.add_argument(
        "-t",
        "--tests",
        nargs="*",
        help="Run specific test(s) by name",
    )
    parser.add_argument(
        "-g",
        "--groups",
        nargs="*",
        help="Run test group(s) - tests organized by directory structure",
    )
    parser.add_argument(
        "--keep-alive",
        metavar="ENV",
        help="Keep-alive mode: start ENV and keep it running (for debugging). Does NOT run tests.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List all available tests and exit",
    )
    parser.add_argument(
        "tests_pos",
        nargs="*",
        help="Run specific test(s) by name or path (positional, optional)",
    )
    return parser.parse_args(argv[1:])


def normalize_test_names(raw_tests: list[str]) -> frozenset[str]:
    """
    Normalize CLI test selectors.

    Accepts bare names, filenames, or paths; strips directories and `.py`.
    """
    names: list[str] = []
    for raw in raw_tests:
        base = os.path.basename(raw)
        name, _ = os.path.splitext(base)
        if name:
            names.append(name)
    return frozenset(names)


def filter_tests(
    args: argparse.Namespace, modules: dict[str, str], test_dir: str
) -> dict[str, str]:
    """
    Filter test modules based on command line arguments.

    Args:
        args: Parsed command line arguments
        modules: Dict mapping test names to file paths
        test_dir: Root test directory path

    Returns:
        Filtered dict of test names to paths
    """
    arg_groups = frozenset(args.groups or [])
    # Normalize tests (accept bare names, filenames, or paths)
    arg_tests = normalize_test_names((args.tests or []) + (args.tests_pos or []))
    disabled = disabled_tests()

    filtered = {}
    for test_name, test_path in modules.items():
        # Extract directory structure for grouping
        test_path_parts = os.path.normpath(test_path).split(os.sep)

        # Find the index of the test directory in the path
        try:
            test_dir_idx = next(
                i for i, part in enumerate(test_path_parts) if part == os.path.basename(test_dir)
            )
        except StopIteration:
            # If test_dir not in path, skip this test
            continue

        # Everything between test_dir and the file is a group
        test_groups = frozenset(test_path_parts[test_dir_idx + 1 : -1])

        # Filtering logic:
        # 1. Skip disabled tests
        if test_name in disabled:
            continue

        # 2. If specific tests requested, only include those
        if arg_tests and test_name not in arg_tests:
            continue

        # 3. If groups requested, only include tests in those groups
        if arg_groups and not (arg_groups & test_groups):
            continue

        filtered[test_name] = test_path

    return filtered


def list_tests(modules: dict[str, str], test_dir: str) -> None:
    """
    List all available tests with their groups.

    Args:
        modules: Dict mapping test names to file paths
        test_dir: Root test directory path
    """
    disabled = disabled_tests()

    # Group tests by directory
    grouped_tests: dict[str, list[str]] = {}
    ungrouped_tests: list[str] = []

    for test_name, test_path in sorted(modules.items()):
        test_path_parts = os.path.normpath(test_path).split(os.sep)

        try:
            test_dir_idx = next(
                i for i, part in enumerate(test_path_parts) if part == os.path.basename(test_dir)
            )
        except StopIteration:
            continue

        groups = test_path_parts[test_dir_idx + 1 : -1]

        if groups:
            group_key = "/".join(groups)
            if group_key not in grouped_tests:
                grouped_tests[group_key] = []
            status = " (disabled)" if test_name in disabled else ""
            grouped_tests[group_key].append(f"  - {test_name}{status}")
        else:
            status = " (disabled)" if test_name in disabled else ""
            ungrouped_tests.append(f"  - {test_name}{status}")

    print("\nAvailable tests:")
    print("=" * 60)

    if ungrouped_tests:
        print("\nRoot tests:")
        for test in ungrouped_tests:
            print(test)

    for group, tests in sorted(grouped_tests.items()):
        print(f"\nGroup: {group}")
        for test in tests:
            print(test)

    print(f"\nTotal: {len(modules)} tests")
    if disabled:
        print(f"Disabled: {len(disabled)} tests")
    print()


def main(argv: list[str]) -> int:
    """Main entry point."""
    args = parse_args(argv)
    setup_logging()

    root_dir = os.path.dirname(os.path.abspath(__file__))
    test_dir = os.path.join(root_dir, "tests")

    # Handle --list mode
    if args.list:
        modules = flexitest.runtime.scan_dir_for_modules(test_dir)
        list_tests(modules, test_dir)
        return 0

    # Create factories
    factories: dict[ServiceType, flexitest.Factory] = {
        ServiceType.AlpenClient: AlpenClientFactory(range(30303, 30503)),
        ServiceType.Bitcoin: BitcoinFactory(range(18443, 18543)),
        ServiceType.Strata: StrataFactory(range(19443, 19543)),
    }

    # Define global environments
    global_envs: dict[str, flexitest.EnvConfig] = {
        "basic": StrataEnvConfig(pre_generate_blocks=110),
        # Alpen-client environments
        "alpen_client": AlpenClientEnv(),  # admin_addPeer, no discovery
        "alpen_client_discovery": AlpenClientEnv(enable_discovery=True, pure_discovery=True),
        "alpen_client_multi": AlpenClientEnv(fullnode_count=3),
        "alpen_client_mesh": AlpenClientEnv(
            fullnode_count=5,
            enable_discovery=True,
            pure_discovery=True,
            mesh_bootnodes=True,
        ),
    }

    # Set up test runtime
    datadir = flexitest.create_datadir_in_workspace(os.path.join(root_dir, "_dd"))
    runtime = TestRuntimeWithLogging(global_envs, datadir, factories)

    # Handle keep-alive mode
    if args.keep_alive:
        if args.keep_alive not in global_envs:
            print(f"Error: Unknown environment '{args.keep_alive}'")
            print(f"Available environments: {', '.join(global_envs.keys())}")
            return 1

        test_class = load_keepalive_test(args.keep_alive, test_dir)
        runtime.prepare_test(KEEP_ALIVE_TEST_NAME, test_class)
        tests = [KEEP_ALIVE_TEST_NAME]
    else:
        # Discover and filter tests
        modules = scan_dir_for_modules(test_dir)
        filtered_modules = filter_tests(args, modules, test_dir)

        if not filtered_modules:
            print("No tests matched the specified filters.")
            if args.tests or args.groups:
                print("\nUse --list to see available tests and groups.")
            return 1

        tests = load_candidate_modules(filtered_modules)
        runtime.prepare_registered_tests()

    # Run tests
    results = runtime.run_tests(tests)

    # Save and display results
    runtime.save_json_file("results.json", results)
    flexitest.dump_results(results)

    # Exit with error if any test failed
    flexitest.fail_on_error(results)

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
