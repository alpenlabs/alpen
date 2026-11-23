import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import release_sp1 as sp1
import release_sp1_s3 as mirror


class ReleaseAssetsTest(unittest.TestCase):
    sha = "a" * 40

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.output = root / "release-assets"
        self.params = root / sp1.PARAMS.name
        self.params.write_bytes(sp1.PARAMS.read_bytes())
        for name in sp1.GUEST_FILES:
            (root / name).write_bytes(name.encode())
        self.enterContext(
            patch.multiple(sp1, PARAMS=self.params, ASSETS=self.output, GENERATED=root)
        )
        self.enterContext(
            patch.dict(
                sp1.os.environ,
                REUSED="",
                SP1_VERSION="v6.10.0",
                GITHUB_REPOSITORY="alpenlabs/alpen",
            )
        )
        sp1.stage("v1", self.sha)

    def test_reuses_verified_bundle_for_another_tag(self):
        self.assertEqual(
            (self.output / self.params.name).read_bytes(), self.params.read_bytes()
        )
        assets = [{"name": path.name} for path in self.output.iterdir()]
        source = {"assets": assets, "isImmutable": True, "isDraft": False}
        with (
            patch.object(sp1.subprocess, "check_output", return_value="v1\nv2\n"),
            patch.object(sp1, "release", side_effect=[None, source]),
            patch.object(sp1, "gh") as download,
            patch.object(sp1, "set_outputs") as outputs,
        ):
            sp1.reuse("v2", self.sha)
            download.assert_any_call(
                "attestation",
                "verify",
                str(self.output / sp1.MANIFEST),
                "--repo",
                "alpenlabs/alpen",
                "--signer-workflow",
                "alpenlabs/alpen/.github/workflows/release-sp1.yml",
                "--source-digest",
                self.sha,
                "--source-ref",
                "refs/tags/v1",
            )
            outputs.assert_called_once_with(found="true")
        with patch.dict(sp1.os.environ, REUSED="true"):
            sp1.stage("v2", self.sha)
        sp1.verify("v2", self.sha)
        (self.output / "guest-checkpoint.elf").write_bytes(b"corrupted")
        with self.assertRaisesRegex(SystemExit, "checksums"):
            sp1.verify("v2", self.sha)

    def test_unattested_release_cannot_supply_build_outputs(self):
        source = {
            "assets": [{"name": p.name} for p in self.output.iterdir()],
            "isImmutable": True,
            "isDraft": False,
        }
        with (
            patch.object(sp1.subprocess, "check_output", return_value="v1"),
            patch.object(sp1, "release", side_effect=[source, None, source]),
            patch.object(sp1, "download"),
            patch.object(
                sp1,
                "verify_provenance",
                side_effect=subprocess.CalledProcessError(1, "verify"),
            ),
            patch.object(sp1, "set_outputs") as outputs,
        ):
            with self.assertRaises(subprocess.CalledProcessError):
                sp1.reuse("v1", self.sha)
            sp1.reuse("v2", self.sha)
            outputs.assert_not_called()
            self.assertFalse(self.output.exists())

    def test_rejects_changed_runtime_params_before_publication(self):
        self.params.write_bytes(self.params.read_bytes() + b"\n")
        with (
            patch.object(sp1, "gh") as gh,
            self.assertRaisesRegex(SystemExit, "params differ"),
        ):
            sp1.publish("v1", self.sha)
        gh.assert_not_called()

    def test_resumes_partial_draft_and_preserves_published_assets(self):
        files = [
            {
                "name": p.name,
                "digest": "sha256:" + hashlib.sha256(p.read_bytes()).hexdigest(),
            }
            for p in self.output.iterdir()
        ]
        complete = {"isDraft": True, "assets": files}
        partial = {"isDraft": True, "assets": files[1:]}
        with (
            patch.object(sp1, "release", side_effect=[partial, complete]),
            patch.object(sp1, "gh", return_value=self.sha) as gh,
        ):
            sp1.publish("v1", self.sha)
            gh.assert_any_call(
                "release", "upload", "v1", str(self.output / files[0]["name"])
            )
            gh.assert_any_call("release", "edit", "v1", "--verify-tag", "--draft=false")
        complete["isDraft"] = False
        with (
            patch.object(sp1, "release", return_value=complete),
            patch.object(sp1, "gh", return_value=self.sha) as gh,
        ):
            sp1.publish("v1", self.sha)
            gh.assert_called_once()
            files[0]["digest"] = "sha256:conflict"
            with self.assertRaisesRegex(SystemExit, "differs"):
                sp1.publish("v1", self.sha)
            self.assertEqual(gh.call_count, 2)


class ReleaseMirrorTest(unittest.TestCase):
    def test_fetch_uses_attestations_and_the_released_commit_params(self):
        info = {"isDraft": False, "isImmutable": True}
        with (
            patch.object(mirror, "release", return_value=info),
            patch.object(mirror, "gh", return_value="abc\n") as gh,
            patch.object(mirror, "download") as download,
            patch.object(
                subprocess, "check_output", return_value=b"tagged params"
            ) as get,
            patch.object(mirror, "verify_provenance") as provenance,
            patch.object(mirror, "verify") as verify,
        ):
            mirror.fetch("v1")
            download.assert_called_once_with("v1")
            provenance.assert_called_once_with("v1", "abc")
            for name in (*sp1.FILES, sp1.MANIFEST):
                gh.assert_any_call(
                    "release", "verify-asset", "v1", str(sp1.ASSETS / name)
                )
            self.assertTrue(get.call_args.args[0][-1].endswith("?ref=abc"))
            verify.assert_called_once_with("v1", "abc", params=b"tagged params")
            info["isImmutable"] = False
            with self.assertRaisesRegex(SystemExit, "immutable"):
                mirror.fetch("v1")
            download.assert_called_once()

    def test_upload_resume_alias_and_conflict(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            files = {name: name.encode() for name in sp1.FILES}
            for name, data in files.items():
                (assets / name).write_bytes(data)
            digests = {
                name: hashlib.sha256(data).hexdigest() for name, data in files.items()
            }
            manifest = {
                "release_tag": "v1",
                "commit": "a" * 40,
                "sp1_version": "v6",
                "sha256": digests,
            }

            def stage(tag):
                manifest["release_tag"] = tag
                (assets / sp1.MANIFEST).write_text(json.dumps(manifest))

            prefix = f"elfs/alpen/aaaaaaaa-{digests[sp1.PARAMS.name][:8]}/"
            first = prefix + sp1.FILES[0]
            stored = {first: files[sp1.FILES[0]]}  # Resume an interrupted copy.

            def put(path, bucket, key):
                self.assertNotIn(key, stored)
                stored[key] = path.read_bytes()

            def list_objects(*args):
                prefix_arg = args[args.index("--prefix") + 1]
                return json.dumps(
                    {
                        "Contents": [
                            {"Key": key} for key in stored if key.startswith(prefix_arg)
                        ]
                    }
                )

            def read(args):
                return stored[args[3].removeprefix("s3://bucket/")]

            with (
                patch.object(mirror, "ASSETS", assets),
                patch.dict(sp1.os.environ, S3_BUCKET="bucket", S3_PREFIX="elfs/alpen"),
                patch.object(mirror, "aws_output", side_effect=list_objects),
                patch.object(mirror, "s3_put", side_effect=put) as writes,
                patch.object(subprocess, "check_output", side_effect=read),
            ):
                stage("v1")
                mirror.upload("v1")
                alias = "elfs/alpen/v1-aaaaaaa/manifest.json"
                self.assertEqual(list(stored)[-2:], [prefix + "manifest.json", alias])
                reference = json.loads(stored[alias])
                self.assertEqual(reference["bundle"], f"s3://bucket/{prefix}")
                target = reference["manifest"].removeprefix("s3://bucket/")
                self.assertEqual(
                    reference["sha256"], hashlib.sha256(stored[target]).hexdigest()
                )
                for name, data in files.items():
                    self.assertEqual(stored[prefix + name], data)
                writes.reset_mock()
                mirror.upload("v1")
                writes.assert_not_called()
                stage("v2")
                mirror.upload("v2")
                self.assertEqual(writes.call_count, 2)
                second_alias = "elfs/alpen/v2-aaaaaaa/manifest.json"
                self.assertEqual(
                    json.loads(stored[second_alias])["bundle"], reference["bundle"]
                )
                stored[alias] = b"conflict"
                stage("v1")
                writes.reset_mock()
                with self.assertRaisesRegex(
                    SystemExit, "conflicting release reference"
                ):
                    mirror.upload("v1")
                writes.assert_not_called()
                stage("v2")
                self.assertEqual(
                    stored[prefix + "releases/v2/" + sp1.MANIFEST],
                    (assets / sp1.MANIFEST).read_bytes(),
                )
                stored[first] = b"conflict"
                writes.reset_mock()
                with self.assertRaisesRegex(SystemExit, "conflicting S3 object"):
                    mirror.upload("v2")
                writes.assert_not_called()
