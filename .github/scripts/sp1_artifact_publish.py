#!/usr/bin/env python3
"""Helpers for the Publish SP1 Artifacts workflow."""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path


ENVIRONMENTS = ("dev", "staging", "prod")
REF_RE = re.compile(r"^[A-Za-z0-9._/@:-]+$")
VERSION_RE = re.compile(r"^[A-Za-z0-9._-]+$")
WHITESPACE_RE = re.compile(r"\s")

GUESTS = (
    ("guest-checkpoint", "checkpoint"),
    ("guest-alpen-chunk", "alpen_chunk"),
    ("guest-alpen-acct", "alpen_acct"),
)

ARTIFACT_FILES = tuple(
    name
    for guest, _ in GUESTS
    for name in (
        f"{guest}.elf",
        f"{guest}.predicate",
        f"{guest}.vk-hash",
    )
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
    """Env: INPUT_ENV, INPUT_REF."""
    env = os.environ["INPUT_ENV"]
    ref = os.environ.get("INPUT_REF", "")

    validate_env(env)
    if ref:
        if WHITESPACE_RE.search(ref):
            fail("ref must not contain whitespace")
        if not REF_RE.fullmatch(ref):
            fail("ref contains unsupported characters")


def cmd_summarize() -> None:
    """Env: CACHE_ROOT, ARTIFACT_DIR, DEPLOY_ENV, ALPEN_REF, ALPEN_SHA,
    SP1_VERSION, GITHUB_STEP_SUMMARY."""
    cache_root = Path(os.environ["CACHE_ROOT"])
    artifact_dir = Path(os.environ["ARTIFACT_DIR"])
    env = validate_env(os.environ["DEPLOY_ENV"])
    alpen_ref = os.environ["ALPEN_REF"]
    alpen_sha = os.environ["ALPEN_SHA"]
    sp1_version = os.environ["SP1_VERSION"]
    run_id = os.environ.get("GITHUB_RUN_ID", "")

    artifact_dir.mkdir(parents=True, exist_ok=True)

    predicates: dict[str, str] = {}
    vk_hashes: dict[str, str] = {}
    for guest, key in GUESTS:
        guest_cache = cache_root / guest / "cache"
        for suffix in ("elf", "predicate", "vk-hash"):
            src = guest_cache / f"{guest}.{suffix}"
            require_file(src)
            (artifact_dir / src.name).write_bytes(src.read_bytes())

        predicates[key] = (artifact_dir / f"{guest}.predicate").read_text().strip()
        vk_hashes[key] = (artifact_dir / f"{guest}.vk-hash").read_text().strip()

    digests = {
        path.name: sha256_hex(path)
        for path in sorted(artifact_dir.iterdir())
        if path.is_file()
    }

    version = f"{env}-{alpen_sha[:8]}"
    if not VERSION_RE.fullmatch(version):
        fail(f"artifact version is not S3-key-safe: {version!r}")

    manifest = {
        "schema": 1,
        "env": env,
        "version": version,
        "run_id": run_id,
        "sp1_version": sp1_version,
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


def s3_cp(src: Path, dst: str) -> None:
    require_file(src)
    print(f"uploading {src} -> {dst}")
    subprocess.run(["aws", "s3", "cp", str(src), dst, "--no-progress"], check=True)


def cmd_upload() -> None:
    """Env: ARTIFACT_DIR, S3_BUCKET, S3_PREFIX, GITHUB_OUTPUT, GITHUB_STEP_SUMMARY."""
    artifact_dir = Path(os.environ["ARTIFACT_DIR"])
    bucket = os.environ["S3_BUCKET"]
    prefix = os.environ.get("S3_PREFIX", "sp1-artifacts")

    if not bucket:
        fail("S3_BUCKET must be set")

    manifest_path = artifact_dir / "manifest.json"
    require_file(manifest_path)
    manifest = json.loads(manifest_path.read_text())
    version = manifest["version"]
    if not VERSION_RE.fullmatch(version):
        fail(f"artifact version is not S3-key-safe: {version!r}")

    base = f"s3://{bucket}/{prefix}/{version}"
    uris: list[str] = []
    for name in ARTIFACT_FILES:
        dst = f"{base}/{name}"
        s3_cp(artifact_dir / name, dst)
        uris.append(dst)
        if name != "manifest.json":
            sidecar_name = f"{name}.sha256"
            s3_cp(artifact_dir / sidecar_name, f"{dst}.sha256")
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
