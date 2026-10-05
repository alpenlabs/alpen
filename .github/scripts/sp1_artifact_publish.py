#!/usr/bin/env python3
"""Helpers for the Publish SP1 Artifacts workflow."""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import tomllib

ENVIRONMENTS = ("dev", "staging", "testnet", "mainnet")
VERSION_RE = re.compile(r"^[A-Za-z0-9._-]+$")
GITHUB_BLOB_URL_RE = re.compile(r"^https://github\.com/[^/]+/[^/]+/blob/")

GUESTS = (("guest-checkpoint", "checkpoint"),)

ARTIFACT_SUFFIXES = ("elf", "predicate", "vk-hash", "artifact-manifest.json")
ARTIFACT_FILES = tuple(
    f"{guest}.{suffix}" for guest, _ in GUESTS for suffix in ARTIFACT_SUFFIXES
) + ("manifest.json",)


def fail(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)
    sys.exit(1)


def set_outputs(**outputs: str) -> None:
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
        for name, value in outputs.items():
            f.write(f"{name}={value}\n")


def validate_env(env: str) -> str:
    if env not in ENVIRONMENTS:
        fail(f"env must be one of {', '.join(ENVIRONMENTS)} (got {env!r})")
    return env


def validate_runtime_params_url(url: str) -> str:
    if any(character.isspace() for character in url):
        fail("checkpoint runtime params URL must not contain whitespace")
    if not url.startswith("https://"):
        fail("checkpoint runtime params URL must start with https://")
    if len(url) > 2048:
        fail("checkpoint runtime params URL exceeds 2048 chars")
    if GITHUB_BLOB_URL_RE.match(url):
        fail(
            "checkpoint runtime params URL is a GitHub /blob/ page (returns HTML); "
            "use the Raw download URL instead"
        )
    return url


def sha256_hex(path: Path) -> str:
    h = __import__("hashlib").sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(8 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_sha256_sidecar(digest: str, name: str, dest_dir: Path) -> Path:
    sidecar = dest_dir / f"{name}.sha256"
    sidecar.write_text(f"{digest}  {name}\n", encoding="utf-8")
    return sidecar


def require_file(path: Path) -> None:
    if not path.is_file() or path.stat().st_size == 0:
        fail(f"expected artifact missing or empty: {path}")


def cmd_validate() -> None:
    """Env: INPUT_ENV, CHECKPOINT_RUNTIME_PARAMS_URL."""
    validate_env(os.environ["INPUT_ENV"])
    validate_runtime_params_url(os.environ["CHECKPOINT_RUNTIME_PARAMS_URL"])


def cmd_resolve_build() -> None:
    """Env: SOURCE_DIR, GITHUB_OUTPUT."""
    source_dir = Path(os.environ["SOURCE_DIR"])
    with (source_dir / "Cargo.lock").open("rb") as lockfile:
        packages = tomllib.load(lockfile)["package"]

    def locked_version(name: str) -> str:
        versions = {
            package["version"] for package in packages if package["name"] == name
        }
        if len(versions) != 1:
            fail(f"expected one {name} version in {source_dir}/Cargo.lock")
        return versions.pop()

    sha = subprocess.check_output(
        ["git", "-C", str(source_dir), "rev-parse", "HEAD"], text=True
    ).strip()
    # sp1-build's default Docker tag follows its crate version. The runner override
    # must likewise use the selected source's locked runner, not the action's default.
    set_outputs(
        sha=sha,
        sp1_version=f"v{locked_version('sp1-build')}",
        sp1_runner_version=locked_version("sp1-core-executor-runner-binary"),
    )


def cmd_fetch() -> None:
    """Env: CHECKPOINT_RUNTIME_PARAMS_URL, CHECKPOINT_RUNTIME_PARAMS_PATH."""
    url = validate_runtime_params_url(os.environ["CHECKPOINT_RUNTIME_PARAMS_URL"])
    path = Path(os.environ["CHECKPOINT_RUNTIME_PARAMS_PATH"])
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        subprocess.run(
            [
                "curl",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "60",
                "--output",
                str(path),
                "--",
                url,
            ],
            check=True,
        )
    except subprocess.CalledProcessError as err:
        fail(f"failed to fetch checkpoint runtime params (exit {err.returncode})")
    require_file(path)
    try:
        params = json.loads(path.read_text(encoding="utf-8"))
    except (ValueError, UnicodeError) as err:
        fail(f"checkpoint runtime params must be JSON: {err}")
    if not isinstance(params, dict):
        fail("checkpoint runtime params must be a JSON object")
    # The guest builder parses OLRuntimeParams or OLParams and owns schema validation.


def cmd_summarize() -> None:
    """Env: ELF_ROOT, ARTIFACT_DIR, DEPLOY_ENV, ALPEN_REF, ALPEN_SHA,
    SP1_VERSION, CHECKPOINT_RUNTIME_PARAMS_URL, CHECKPOINT_RUNTIME_PARAMS_PATH,
    GITHUB_STEP_SUMMARY."""
    elf_root = Path(os.environ["ELF_ROOT"])
    artifact_dir = Path(os.environ["ARTIFACT_DIR"])
    env = validate_env(os.environ["DEPLOY_ENV"])
    alpen_ref = os.environ["ALPEN_REF"]
    alpen_sha = os.environ["ALPEN_SHA"]
    sp1_version = os.environ["SP1_VERSION"]
    run_id = os.environ.get("GITHUB_RUN_ID", "")
    runtime_params_url = os.environ["CHECKPOINT_RUNTIME_PARAMS_URL"]
    runtime_params_path = Path(os.environ["CHECKPOINT_RUNTIME_PARAMS_PATH"])
    require_file(runtime_params_path)
    runtime_params_sha256 = sha256_hex(runtime_params_path)

    artifact_dir.mkdir(parents=True, exist_ok=True)

    predicates: dict[str, str] = {}
    vk_hashes: dict[str, str] = {}
    for guest, key in GUESTS:
        for suffix in ARTIFACT_SUFFIXES:
            src = elf_root / f"{guest}.{suffix}"
            require_file(src)
            (artifact_dir / src.name).write_bytes(src.read_bytes())

        predicates[key] = (artifact_dir / f"{guest}.predicate").read_text().strip()
        vk_hashes[key] = (artifact_dir / f"{guest}.vk-hash").read_text().strip()

    digests = {
        name: sha256_hex(artifact_dir / name)
        for name in ARTIFACT_FILES
        if name != "manifest.json"
    }

    version = f"{alpen_sha[:8]}-{runtime_params_sha256[:8]}"
    if not VERSION_RE.fullmatch(version):
        fail(f"artifact version is not S3-key-safe: {version!r}")

    manifest = {
        "schema": 1,
        "env": env,
        "version": version,
        "run_id": run_id,
        "sp1_version": sp1_version,
        "checkpoint_runtime_params": {
            "url": runtime_params_url,
            "sha256": runtime_params_sha256,
        },
        "alpen": {
            "ref": alpen_ref,
            "sha": alpen_sha,
        },
        "predicates": predicates,
        "vk_hashes": vk_hashes,
        "sha256": digests,
    }
    (artifact_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )

    for name, digest in manifest["sha256"].items():
        write_sha256_sidecar(digest, name, artifact_dir)

    set_outputs(version=version)

    lines = [
        "## SP1 artifact publish",
        "",
        f"- env: `{env}`",
        f"- alpen ref: `{alpen_ref}` @ `{alpen_sha}`",
        f"- SP1 toolchain: `{sp1_version}`",
        f"- runtime params source: `{runtime_params_url}`",
        f"- runtime params file SHA-256: `{runtime_params_sha256}`",
        f"- version: `{version}`",
        "",
        "### Predicates",
        "",
        *(f"- {key}: `{value}`" for key, value in predicates.items()),
        "",
        "### VK hashes",
        "",
        *(f"- {key}: `{value}`" for key, value in vk_hashes.items()),
        "",
        "### SHA-256",
        "",
        "```",
        *(f"{digest}  {name}" for name, digest in digests.items()),
        "```",
        "",
    ]
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a", encoding="utf-8") as f:
        f.write("\n".join(lines))


def s3_put(src: Path, bucket: str, key: str) -> None:
    require_file(src)
    print(f"uploading {src} -> s3://{bucket}/{key}")
    try:
        subprocess.run(
            [
                "aws",
                "s3api",
                "put-object",
                "--bucket",
                bucket,
                "--key",
                key,
                "--body",
                str(src),
                "--if-none-match",
                "*",
            ],
            check=True,
        )
    except subprocess.CalledProcessError as err:
        fail(
            f"failed to upload {src} to s3://{bucket}/{key} (exit {err.returncode}); "
            "SP1 artifact publishes are write-once; existing objects are never overwritten"
        )


def cmd_upload() -> None:
    """Env: ARTIFACT_DIR, S3_BUCKET, S3_PREFIX, GITHUB_OUTPUT, GITHUB_STEP_SUMMARY."""
    artifact_dir = Path(os.environ["ARTIFACT_DIR"])
    bucket = os.environ["S3_BUCKET"]
    prefix = os.environ.get("S3_PREFIX", "elfs/alpen")

    if not bucket:
        fail("S3_BUCKET must be set")

    manifest_path = artifact_dir / "manifest.json"
    require_file(manifest_path)
    manifest = json.loads(manifest_path.read_text())
    version = manifest["version"]
    if not VERSION_RE.fullmatch(version):
        fail(f"artifact version is not S3-key-safe: {version!r}")

    # Validate the entire downloaded bundle before any write-once S3 upload.
    for name in ARTIFACT_FILES:
        path = artifact_dir / name
        require_file(path)
        if name == "manifest.json":
            continue
        digest = sha256_hex(path)
        if manifest.get("sha256", {}).get(name) != digest:
            fail(f"artifact checksum does not match manifest: {name}")
        sidecar = artifact_dir / f"{name}.sha256"
        require_file(sidecar)
        if sidecar.read_text(encoding="utf-8") != f"{digest}  {name}\n":
            fail(f"artifact checksum sidecar does not match: {name}")

    base_key = f"{prefix}/{version}"
    base = f"s3://{bucket}/{base_key}"
    uris: list[str] = []
    for name in ARTIFACT_FILES:
        key = f"{base_key}/{name}"
        dst = f"s3://{bucket}/{key}"
        s3_put(artifact_dir / name, bucket, key)
        uris.append(dst)
        if name != "manifest.json":
            sidecar_name = f"{name}.sha256"
            s3_put(artifact_dir / sidecar_name, bucket, f"{key}.sha256")
            uris.append(f"{dst}.sha256")

    set_outputs(version=version, s3_base=base)

    lines = [
        "### S3 upload",
        "",
        f"- location: `{base}/`",
        "",
        *(f"- `{uri}`" for uri in uris),
        "",
    ]
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a", encoding="utf-8") as f:
        f.write("\n".join(lines))


COMMANDS = {
    "validate": cmd_validate,
    "resolve-build": cmd_resolve_build,
    "fetch": cmd_fetch,
    "summarize": cmd_summarize,
    "upload": cmd_upload,
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=sorted(COMMANDS))
    args = parser.parse_args()
    COMMANDS[args.command]()


if __name__ == "__main__":
    main()
