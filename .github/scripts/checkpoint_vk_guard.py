#!/usr/bin/env python3
"""Helpers for the Checkpoint VK guard workflow.

The workflow builds the SP1 checkpoint guest at a PR's base and at its merge commit, with the
Docker build that publish-sp1-artifacts.yml uses and the fixture runtime params, and fails when
the two verifying keys differ. Run this script from a copy outside the checkout: `build base`
checks out the base commit, which may not have it.
"""

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

import tomllib
from sp1_artifact_publish import fail, require_file, sha256_hex

BUILDER = "strata-sp1-guest-builder"
GUEST = "guest-checkpoint"
GUEST_MANIFEST = Path("provers/sp1/guest-checkpoint/Cargo.toml")
GENERATED_DIR = Path("provers/sp1/generated")
RUNTIME_PARAMS = Path(".github/fixtures/checkpoint-runtime-params.json")
WORKFLOW = Path(".github/workflows/checkpoint-vk-guard.yml")
SIDES = ("base", "head")
# Guest inputs besides the local crates the guest compiles. The path crates inherit
# `workspace = true` dependencies from the root manifest, and Cargo in the Docker build reads
# the root Cargo config.
SHARED_INPUT_DIRS = ("provers/sp1/", ".cargo/")
SHARED_INPUT_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    str(RUNTIME_PARAMS),
)


def run(*args: str, env: dict[str, str] | None = None, capture: bool = False) -> str:
    try:
        return subprocess.run(
            args,
            check=True,
            env=env,
            stdout=subprocess.PIPE if capture else None,
            text=True,
            errors="replace",
        ).stdout
    except subprocess.CalledProcessError as err:
        fail(f"{' '.join(args)} failed (exit {err.returncode})")


def side_dir(side: str) -> Path:
    return Path(os.environ["RUNNER_TEMP"]) / "checkpoint-vk-guard" / side


def annotate(level: str, title: str, message: str) -> None:
    message = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::{level} title={title}::{message}")


def locked_version(name: str) -> str:
    with Path("Cargo.lock").open("rb") as lockfile:
        packages = tomllib.load(lockfile)["package"]
    versions = {package["version"] for package in packages if package["name"] == name}
    if len(versions) != 1:
        fail(f"expected one {name} version in Cargo.lock")
    return versions.pop()


def docker_image(tag: str) -> str:
    """Returns the digest of the SP1 image sp1-build pulled, or the tag if it is unknown."""
    image = f"ghcr.io/succinctlabs/sp1:{tag}"
    inspect = ["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", image]
    result = subprocess.run(inspect, check=False, stdout=subprocess.PIPE, text=True)
    digests = json.loads(result.stdout or "null") if result.returncode == 0 else None
    return digests[0] if digests else image


def guest_dirs() -> list[str]:
    """Returns the repository directories of the local crates the guest compiles."""
    metadata_args = (
        "--locked",
        "--format-version=1",
        f"--manifest-path={GUEST_MANIFEST}",
    )
    metadata = json.loads(run("cargo", "metadata", *metadata_args, capture=True))
    root = Path.cwd().resolve()
    return sorted(
        Path(package["manifest_path"]).parent.relative_to(root).as_posix()
        for package in metadata["packages"]
        if package["source"] is None
    )


def cmd_build(side: str) -> None:
    """Env: BASE_SHA, HEAD_SHA, RUNNER_TEMP."""
    sha = os.environ[f"{side.upper()}_SHA"]
    out_dir = side_dir(side)
    run("git", "checkout", "--quiet", "--detach", sha)
    try:
        # Drop the previous build's artifacts so they cannot pass for this one, and make the
        # builder's build script rerun: it does not watch every guest input, such as the root
        # Cargo.toml.
        shutil.rmtree(GENERATED_DIR, ignore_errors=True)
        run("cargo", "clean", "--release", f"--package={BUILDER}")
        build_env = {
            **os.environ,
            "BUILD_ELF": "1",
            "BUILD_VKEY": "1",
            "CHECKPOINT_RUNTIME_PARAMS_PATH": str(RUNTIME_PARAMS.resolve()),
        }
        build_args = (
            "--locked",
            "--release",
            f"--package={BUILDER}",
            "--features=docker-build",
        )
        run("cargo", "build", *build_args, env=build_env)

        out_dir.mkdir(parents=True, exist_ok=True)
        for suffix in ("elf", "vk-hash", "artifact-manifest.json"):
            src = GENERATED_DIR / f"{GUEST}.{suffix}"
            require_file(src)
            shutil.copyfile(src, out_dir / src.name)
        # sp1-build's Docker image tag follows its crate version.
        sp1_version = f"v{locked_version('sp1-build')}"
        build = {
            "sha": sha,
            "sp1_version": sp1_version,
            "docker_image": docker_image(sp1_version),
            "guest_dirs": guest_dirs(),
        }
        (out_dir / "build.json").write_text(json.dumps(build, indent=2) + "\n")
    finally:
        run("git", "checkout", "--quiet", "--detach", os.environ["HEAD_SHA"])


def load_side(side: str) -> dict:
    out_dir = side_dir(side)
    for name in (f"{GUEST}.elf", f"{GUEST}.vk-hash", "build.json"):
        require_file(out_dir / name)
    try:
        manifest = json.loads((out_dir / f"{GUEST}.artifact-manifest.json").read_text())
        build = json.loads((out_dir / "build.json").read_text())
        spec = manifest["spec"]
        program_id = manifest["program_id"]
        runtime_params_hash = manifest["runtime_params_hash"]
    except (OSError, ValueError, KeyError, TypeError) as err:
        fail(f"invalid {side} build output in {out_dir}: {err}")
    vk_hash = (out_dir / f"{GUEST}.vk-hash").read_text().strip()
    if manifest.get("schema") != 1 or vk_hash != f"0x{program_id}":
        fail(f"{side} artifact manifest does not match its vk-hash")
    return {
        **build,
        "vk_hash": vk_hash,
        "spec": spec,
        "runtime_params_hash": runtime_params_hash,
        "elf_sha256": sha256_hex(out_dir / f"{GUEST}.elf"),
    }


def changed_guest_inputs(base_sha: str, head_sha: str, dirs: set[str]) -> list[str]:
    diff_args = ("-z", "--name-only", "--no-renames", base_sha, head_sha)
    diff = run("git", "diff", *diff_args, capture=True)
    prefixes = (*SHARED_INPUT_DIRS, *(f"{d}/" for d in sorted(dirs)))
    return [
        name
        for name in diff.split("\0")
        if name and (name.startswith(prefixes) or name in SHARED_INPUT_FILES)
    ]


def path_filters() -> list[str]:
    """Reads the `paths:` list of this workflow's `pull_request` trigger."""
    filters: list[str] = []
    indent = None
    for line in WORKFLOW.read_text().splitlines():
        stripped = line.strip()
        if indent is None:
            if stripped == "paths:":
                indent = len(line) - len(line.lstrip())
            continue
        if not stripped or stripped.startswith("#"):
            continue
        if not stripped.startswith("- ") or len(line) - len(line.lstrip()) < indent:
            break
        filters.append(stripped[2:].split(" #")[0].strip().strip("\"'"))
    if not filters:
        fail(f"no paths filter found in {WORKFLOW}")
    return filters


def covers(pattern: str, directory: str) -> bool:
    prefix = pattern.removesuffix("/**")
    return pattern == "**" or (
        pattern.endswith("/**")
        and (directory == prefix or directory.startswith(f"{prefix}/"))
    )


def code(text: object) -> str:
    """Renders text as inline code. File names come from the PR, so escape what could end it."""
    escaped = "".join(c if c.isprintable() else repr(c)[1:-1] for c in str(text))
    return "`" + escaped.replace("`", "'") + "`"


def cmd_compare() -> None:
    """Env: RUNNER_TEMP, GITHUB_STEP_SUMMARY."""
    base, head = (load_side(side) for side in SIDES)
    changed = changed_guest_inputs(
        base["sha"], head["sha"], set(base["guest_dirs"]) | set(head["guest_dirs"])
    )
    filters = path_filters()
    uncovered = [
        d for d in head["guest_dirs"] if not any(covers(p, d) for p in filters)
    ]
    vk_changed = base["vk_hash"] != head["vk_hash"]

    if vk_changed:
        verdict = [
            "> [!CAUTION]",
            "> This PR changes the checkpoint VK; if intended, it needs a new spec.",
            ">",
            "> This check is advisory and stays red for an intended change.",
        ]
    else:
        verdict = ["This PR does not change the checkpoint VK."]
    rows = (
        ("Commit", "sha"),
        ("VK hash", "vk_hash"),
        ("Proved spec", "spec"),
        ("Runtime params hash", "runtime_params_hash"),
        ("ELF SHA-256", "elf_sha256"),
        ("SP1", "sp1_version"),
        ("Docker image", "docker_image"),
    )
    lines = [
        "## Checkpoint VK guard",
        "",
        *verdict,
        "",
        "| | Base | This PR (merge commit) |",
        "| --- | --- | --- |",
        *(
            f"| {label} | {code(base[key])} | {code(head[key])} |"
            for label, key in rows
        ),
        "",
        (
            "Both sides use the Docker build that `publish-sp1-artifacts.yml` uses and the "
            f"runtime params in {code(RUNTIME_PARAMS)}. Each network embeds its own params and "
            "so has its own key, but a code change moves every network's key alike."
        ),
        "",
    ]
    if base["runtime_params_hash"] != head["runtime_params_hash"]:
        lines += ["The fixture runtime params differ between the two sides.", ""]
    lines += ["### Guest inputs this PR changes", ""]
    if changed:
        lines += [f"- {code(name)}" for name in changed]
    elif vk_changed:
        lines.append(
            "None that this check tracks, so an untracked input or a nondeterministic build "
            "moved the key."
        )
    else:
        lines.append("None.")
    lines.append("")
    if uncovered:
        lines += [
            "### Path filter",
            "",
            f"These guest crates are missing from the `paths` filter of {code(WORKFLOW)}:",
            "",
            *(f"- {code(f'{d}/**')}" for d in uncovered),
            "",
        ]
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a", encoding="utf-8") as summary:
        summary.write("\n".join(lines))

    if vk_changed:
        annotate(
            "error",
            "Checkpoint VK changed",
            f"This PR changes the checkpoint VK from {base['vk_hash']} to "
            f"{head['vk_hash']}; if intended, it needs a new spec. See the job summary.",
        )
    elif base["elf_sha256"] != head["elf_sha256"]:
        annotate(
            "notice", "Checkpoint ELF changed", "The ELF changed but the VK did not."
        )
    for directory in uncovered:
        annotate(
            "error",
            "Checkpoint VK guard path filter",
            f"{directory} is compiled into the guest but missing from the paths filter of "
            f'{WORKFLOW}; add "{directory}/**".',
        )
    if vk_changed or uncovered:
        raise SystemExit(1)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("build").add_argument("side", choices=SIDES)
    subcommands.add_parser("compare")
    args = parser.parse_args()
    if args.command == "build":
        cmd_build(args.side)
    else:
        cmd_compare()


if __name__ == "__main__":
    main()
