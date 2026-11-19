"""Build-side staging and write-once GitHub Release publication for checkpoint guests."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import quote

from sp1_artifact_publish import ARTIFACT_FILES, require_file, set_outputs, sha256_hex

PARAMS = Path("provers/sp1/release-params/checkpoint-runtime-params.json")
GENERATED = Path("provers/sp1/generated")
ASSETS = Path("release-assets")
MANIFEST = "proving-manifest.json"
GUEST_FILES = tuple(name for name in ARTIFACT_FILES if name != "manifest.json")
FILES = (*GUEST_FILES, PARAMS.name)


def gh(*args: str) -> str:
    return subprocess.check_output(["gh", *args], text=True)


def release(tag: str) -> dict | None:
    result = subprocess.run(
        ["gh", "release", "view", tag, "--json", "assets,isDraft,isImmutable"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode and result.stderr.strip() == "release not found":
        return None
    if result.returncode:
        raise SystemExit(result.stderr.strip())
    return json.loads(result.stdout)


def checksums() -> dict[str, str]:
    for name in FILES:
        require_file(ASSETS / name)
    if (ASSETS / PARAMS.name).read_bytes() != PARAMS.read_bytes():
        raise SystemExit("release params differ from the tagged build input")
    return {name: sha256_hex(ASSETS / name) for name in FILES}


def verify(tag: str, commit: str) -> None:
    manifest = json.loads((ASSETS / MANIFEST).read_bytes())
    if (manifest["release_tag"], manifest["commit"], manifest["sha256"]) != (
        tag,
        commit,
        checksums(),
    ):
        raise SystemExit(
            f"release bundle for {tag} has conflicting provenance or checksums"
        )


def reuse(tag: str, commit: str) -> None:
    tags = subprocess.check_output(
        ["git", "tag", "--points-at", commit, "--list", "v*"], text=True
    )
    for candidate in dict.fromkeys([tag, *tags.splitlines()]):
        info = release(candidate)
        if info is None or (candidate != tag and not info["isImmutable"]):
            continue
        names = {asset["name"] for asset in info["assets"]}
        if not names.issuperset((*FILES, MANIFEST)):
            if candidate == tag and not info["isDraft"]:
                raise SystemExit(
                    f"{tag} is already published with an incomplete proving bundle"
                )
            continue
        args = [arg for name in (*FILES, MANIFEST) for arg in ("--pattern", name)]
        gh("release", "download", candidate, "--dir", str(ASSETS), *args)
        verify(candidate, commit)
        print(f"Reusing verified checkpoint assets from {candidate}")
        set_outputs(found="true")
        return


def stage(tag: str, commit: str) -> None:
    ASSETS.mkdir(exist_ok=True)
    if os.environ["REUSED"] != "true":
        for name in GUEST_FILES:
            shutil.copyfile(GENERATED / name, ASSETS / name)
        shutil.copyfile(PARAMS, ASSETS / PARAMS.name)
    manifest = {
        "release_tag": tag,
        "commit": commit,
        "sp1_version": os.environ["SP1_VERSION"],
        "sha256": checksums(),
    }
    (ASSETS / MANIFEST).write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )


def missing_assets(info: dict) -> list[str]:
    published = {asset["name"]: asset["digest"] for asset in info["assets"]}
    missing = []
    for name in (*FILES, MANIFEST):
        if name not in published:
            missing.append(str(ASSETS / name))
        elif published[name] != f"sha256:{sha256_hex(ASSETS / name)}":
            raise SystemExit(f"existing release asset differs: {name}")
    return missing


def publish(tag: str, commit: str) -> None:
    verify(tag, commit)
    ref = quote(f"refs/tags/{tag}", safe="")
    if (
        gh("api", f"repos/{{owner}}/{{repo}}/commits/{ref}", "--jq", ".sha").strip()
        != commit
    ):
        raise SystemExit(f"{tag} moved during the build")
    info = release(tag)
    if info is None:
        args = ["--prerelease"] if "-" in tag else []
        gh(
            "release",
            "create",
            tag,
            "--verify-tag",
            "--draft",
            "--generate-notes",
            *args,
        )
        info = release(tag)
    if info is None:
        raise SystemExit(f"could not find the draft release for {tag}")

    missing = missing_assets(info)
    if not info["isDraft"]:
        if missing:
            raise SystemExit(f"published release {tag} is missing assets: {missing}")
        return
    if missing:
        gh("release", "upload", tag, *missing)
        uploaded = release(tag)
        if uploaded is None or missing_assets(uploaded):
            raise SystemExit(f"incomplete release upload for {tag}")
    gh("release", "edit", tag, "--verify-tag", "--draft=false")


if __name__ == "__main__":
    command = {"reuse": reuse, "stage": stage, "publish": publish}[sys.argv[1]]
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    command(os.environ["GITHUB_REF_NAME"], head)
