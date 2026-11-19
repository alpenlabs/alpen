import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import release_sp1 as sp1


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
        self.enterContext(patch.dict(sp1.os.environ, REUSED="", SP1_VERSION="v6.10.0"))
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
            download.assert_called_once()
            outputs.assert_called_once_with(found="true")
        with patch.dict(sp1.os.environ, REUSED="true"):
            sp1.stage("v2", self.sha)
        sp1.verify("v2", self.sha)
        (self.output / "guest-checkpoint.elf").write_bytes(b"corrupted")
        with self.assertRaisesRegex(SystemExit, "checksums"):
            sp1.verify("v2", self.sha)

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
