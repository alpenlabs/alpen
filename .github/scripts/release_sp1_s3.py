"""Copies attested GitHub Release assets into the existing write-once SP1 bundle layout."""

import json
import os
import re
import subprocess
import sys
from urllib.parse import quote

from release_sp1 import (
    ASSETS,
    FILES,
    GUEST_FILES,
    MANIFEST,
    PARAMS,
    download,
    gh,
    release,
    verify,
    verify_provenance,
)
from sp1_artifact_publish import (
    GUESTS,
    artifact_version,
    aws_output,
    s3_put,
    sha256_hex,
    write_sha256_sidecar,
)


def fetch(tag: str) -> None:
    if not re.fullmatch(r"v[A-Za-z0-9._-]+", tag):
        raise SystemExit(f"invalid release tag: {tag}")
    info = release(tag)
    if not info or info["isDraft"] or not info["isImmutable"]:
        raise SystemExit(f"{tag} must be a published immutable release")
    ref = quote(f"refs/tags/{tag}", safe="")
    commit = gh(
        "api", f"repos/{{owner}}/{{repo}}/commits/{ref}", "--jq", ".sha"
    ).strip()
    download(tag)
    verify_provenance(tag, commit)
    for name in (*FILES, MANIFEST):
        gh("release", "verify-asset", tag, str(ASSETS / name))
    # This workflow runs on main, whose params may have changed since the tag.
    params = subprocess.check_output(
        [
            "gh",
            "api",
            "-H",
            "Accept: application/vnd.github.raw+json",
            f"repos/{{owner}}/{{repo}}/contents/{PARAMS}?ref={commit}",
        ]
    )
    verify(tag, commit, params=params)


def upload(tag: str) -> None:
    manifest = json.loads((ASSETS / MANIFEST).read_bytes())
    sha, digests = manifest["commit"], manifest["sha256"]
    if manifest["release_tag"] != tag:
        raise SystemExit("release tag differs from downloaded manifest")
    params_hash = digests[PARAMS.name]
    version = artifact_version(sha, params_hash)
    bucket = os.environ["S3_BUCKET"]
    root = os.environ["S3_PREFIX"]
    prefix = f"{root}/{version}/"

    def listed_keys(key_prefix: str) -> set[str]:
        listing = json.loads(
            aws_output(
                "s3api",
                "list-objects-v2",
                "--bucket",
                bucket,
                "--prefix",
                key_prefix,
                "--output",
                "json",
            )
        )
        return {obj["Key"] for obj in listing.get("Contents", [])}

    existing = listed_keys(prefix)

    def read(key: str) -> bytes:
        return subprocess.check_output(
            ["aws", "s3", "cp", f"s3://{bucket}/{key}", "-", "--only-show-errors"]
        )

    # Older manual bundles use the same key. Verify their full identity before adding files.
    marker = prefix + "manifest.json"
    if marker in existing:
        stored = json.loads(read(marker))
        if (
            stored["alpen"]["sha"] != sha
            or stored["checkpoint_runtime_params"]["sha256"] != params_hash
            or any(stored["sha256"].get(name) != digests[name] for name in GUEST_FILES)
        ):
            raise SystemExit(f"conflicting bundle manifest at {marker}")

    objects = {prefix + name: ASSETS / name for name in FILES}
    for name in FILES:
        sidecar = write_sha256_sidecar(digests[name], name, ASSETS)
        objects[prefix + sidecar.name] = sidecar
    # The release manifest includes its tag. Several tags can share the same canonical files.
    release_key = f"{prefix}releases/{tag}/{MANIFEST}"
    objects[release_key] = ASSETS / MANIFEST

    alias_key = f"{root}/{tag}-{sha[:7]}/manifest.json"
    alias = ASSETS / "release-reference.json"
    reference = {
        "bundle": f"s3://{bucket}/{prefix}",
        "manifest": f"s3://{bucket}/{release_key}",
        "sha256": sha256_hex(ASSETS / MANIFEST),
    }
    alias.write_text(json.dumps(reference, indent=2, sort_keys=True) + "\n")
    alias_exists = alias_key in listed_keys(alias_key)
    if alias_exists and read(alias_key) != alias.read_bytes():
        raise SystemExit(f"conflicting release reference at {alias_key}")

    for key, path in objects.items():
        if key in existing and read(key) != path.read_bytes():
            raise SystemExit(f"conflicting S3 object at {key}")
    for key, path in objects.items():
        if key not in existing:
            s3_put(path, bucket, key)

    if marker not in existing:
        canonical = {
            "schema": 1,
            "version": version,
            "alpen": {"ref": tag, "sha": sha},
            "sp1_version": manifest["sp1_version"],
            "checkpoint_runtime_params": {"file": PARAMS.name, "sha256": params_hash},
            "predicates": {
                key: (ASSETS / f"{guest}.predicate").read_text().strip()
                for guest, key in GUESTS
            },
            "vk_hashes": {
                key: (ASSETS / f"{guest}.vk-hash").read_text().strip()
                for guest, key in GUESTS
            },
            "sha256": digests,
        }
        path = ASSETS / "manifest.json"
        path.write_text(json.dumps(canonical, indent=2, sort_keys=True) + "\n")
        s3_put(path, bucket, marker)
    if not alias_exists:
        s3_put(alias, bucket, alias_key)
    print(f"Release reference: s3://{bucket}/{alias_key}")


if __name__ == "__main__":
    {"fetch": fetch, "upload": upload}[sys.argv[1]](os.environ["RELEASE_TAG"])
