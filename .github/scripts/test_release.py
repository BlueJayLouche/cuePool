import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / release.ARTIFACTS[0]).write_bytes(b"payload" + b"koly" + bytes(508))
        (self.root / release.ARTIFACTS[2]).write_bytes(bytes.fromhex("d0cf11e0a1b11ae1") + b"msi")
        with zipfile.ZipFile(self.root / release.ARTIFACTS[1], "w") as z:
            z.writestr("cuepool.exe", b"exe")
            z.writestr("avcodec.dll", b"dll")

    def test_failed_or_skipped_packaging_never_mutates_github(self):
        for status in ("failure", "cancelled", "skipped", ""):
            with self.subTest(status=status), patch.object(release, "find_release", return_value=None), patch.object(release, "api") as api:
                with self.assertRaisesRegex(ValueError, "BOTH platform"):
                    release.publish("v0.12.3", "a" * 40, self.root, status, "success")
                api.assert_not_called()
        with self.assertRaisesRegex(ValueError, "verification"):
            release.validate_artifacts(self.root, "success", "failure")

    def test_missing_duplicate_and_invalid_payloads_fail_before_publication(self):
        (self.root / release.ARTIFACTS[2]).unlink()
        with self.assertRaisesRegex(ValueError, "Missing"):
            release.validate_artifacts(self.root, "success", "success")
        (self.root / release.ARTIFACTS[2]).write_bytes(b"wrong")
        with self.assertRaisesRegex(ValueError, "MSI"):
            release.validate_artifacts(self.root, "success", "success")
        extra = self.root / "duplicate"
        extra.mkdir()
        (extra / release.ARTIFACTS[0]).write_bytes(b"duplicate")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            release.validate_artifacts(self.root, "success", "success")

    def test_tag_is_immutable_and_retry_is_noop(self):
        with patch.object(release, "api", return_value={"object": {"type": "commit", "sha": "a" * 40}}) as api:
            release.ensure_tag("v0.12.3", "a" * 40)
            self.assertEqual(api.call_count, 1)
            with self.assertRaisesRegex(ValueError, "Refusing to move"):
                release.ensure_tag("v0.12.3", "b" * 40)
        for bad in ("v1.2", "main", "v1.2.3\n", "--draft", "v1.2.3/extra"):
            with self.assertRaises(ValueError):
                release.tag_name(bad)

    def test_draft_lookup_avoids_duplicate_release(self):
        draft = {"tag_name": "v0.12.3", "draft": True}
        with patch.object(release, "api", side_effect=[None, [draft]]) as api:
            self.assertEqual(release.find_release("v0.12.3"), draft)
            self.assertTrue(all(call.args[1:] == () for call in api.call_args_list))

    def test_upload_and_readback_precede_publication_and_receipt(self):
        events = []
        draft = {"tag_name": "v0.12.3", "draft": True, "body": "Notes\n\n" + release.MACOS_NOTE + "\n\n<!-- cuepool-source: " + "a" * 40 + " -->\n"}
        public = {**draft, "draft": False, "published_at": "2026-09-08T00:00:00Z"}
        with patch.object(release, "find_release", return_value=draft), \
             patch.object(release, "ensure_tag"), patch.object(release, "release_notes", return_value="Notes"), \
             patch.object(release, "should_promote_latest", return_value=True), \
             patch.object(release, "run", side_effect=lambda *args: events.append(args[2])), \
             patch.object(release, "verify_remote_assets", side_effect=lambda *args: events.append("readback")), \
             patch.object(release, "api", return_value=public), \
             patch.object(release, "record_publication", side_effect=lambda *args: events.append("receipt")):
            release.publish("v0.12.3", "a" * 40, self.root, "success", "success")
        self.assertEqual(events, ["edit", "upload", "readback", "edit", "receipt"])

    def test_failed_readback_keeps_draft_private(self):
        draft = {"tag_name": "v0.12.3", "draft": True, "body": "Notes\n\n" + release.MACOS_NOTE + "\n\n<!-- cuepool-source: " + "a" * 40 + " -->\n"}
        with patch.object(release, "find_release", return_value=draft), \
             patch.object(release, "ensure_tag"), patch.object(release, "release_notes", return_value="Notes"), \
             patch.object(release, "run") as run, \
             patch.object(release, "verify_remote_assets", side_effect=ValueError("digest mismatch")), \
             patch.object(release, "record_publication") as receipt:
            with self.assertRaisesRegex(ValueError, "digest"):
                release.publish("v0.12.3", "a" * 40, self.root, "success", "success")
            self.assertEqual([call.args[2] for call in run.call_args_list], ["edit", "upload"])
            receipt.assert_not_called()

    def test_published_retry_never_uploads_or_creates_release(self):
        with patch.object(release, "find_release", return_value={"draft": False}), \
             patch.object(release, "repair_receipt") as repair, patch.object(release, "run") as run:
            release.publish("v0.12.3", "a" * 40, self.root, "success", "success")
            repair.assert_called_once_with("v0.12.3", "a" * 40)
            run.assert_not_called()

    def test_receipt_requires_publication_and_matching_source(self):
        with self.assertRaisesRegex(ValueError, "unpublished"):
            release.record_publication("v0.12.3", "a" * 40, {"draft": True})
        public = {"draft": False, "tag_name": "v0.12.3", "html_url": "https://github.com/BlueJayLouche/cuePool/releases/tag/v0.12.3", "published_at": "today"}
        with patch.object(release, "api", side_effect=[None, {"sha": "receipt-object"}, None]) as api:
            release.record_publication("v0.12.3", "a" * 40, public)
            annotation = api.call_args_list[1].args[2]
            self.assertEqual(json.loads(annotation["message"])["commit"], "a" * 40)
            self.assertEqual(api.call_args_list[2].args[2]["ref"], "refs/tags/published/v0.12.3")

    def test_source_fix_recovers_untagged_candidate_without_another_version_bump(self):
        sha = "b" * 40
        for tag_commit, expected in ((None, "true"), (sha, "true"), ("a" * 40, "false")):
            ref = {"object": {"type": "commit", "sha": tag_commit}} if tag_commit else None
            with self.subTest(tag_commit=tag_commit), \
                 patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/main"}), \
                 patch.object(release, "validate_workspace", return_value="0.13.0"), \
                 patch.object(release, "api", return_value=ref), \
                 patch.object(release, "git", return_value=sha), patch.object(release.subprocess, "run"), \
                 patch.object(release, "find_release", return_value=None), patch.object(release, "output") as output:
                release.resolve_candidate()
                values = output.call_args.args[0]
                self.assertEqual(values["active"], expected)
                self.assertEqual(values["sha"], sha)
                self.assertEqual(values["tag"], "v0.13.0" if expected == "true" else "")

    def test_older_retry_does_not_become_latest(self):
        releases = [
            {"tag_name": "v0.9.0", "draft": False},
            {"tag_name": "v0.13.1", "draft": False},
            {"tag_name": "v0.14.0", "draft": True},
            {"tag_name": "v0.15.0", "draft": False, "prerelease": True},
        ]
        with patch.object(release, "api", return_value=releases):
            self.assertFalse(release.should_promote_latest("v0.13.0"))
            self.assertTrue(release.should_promote_latest("v0.13.2"))
        with patch.object(release, "api", side_effect=[[releases[0]] * 100, [releases[1]]]):
            self.assertFalse(release.should_promote_latest("v0.13.0"))

    def test_reused_draft_gets_canonical_notes_and_attestation(self):
        draft = {"tag_name": "v0.12.3", "draft": True, "body": "Old manually written notes"}
        body = "Notes\n\n" + release.MACOS_NOTE + "\n\n<!-- cuepool-source: " + "a" * 40 + " -->\n"
        normalized = {**draft, "body": body}
        commands = []
        def command(*args):
            commands.append(args)
            if "--notes-file" in args:
                self.assertEqual(Path(args[args.index("--notes-file") + 1]).read_text(), body)
        with patch.object(release, "find_release", side_effect=[draft, draft, normalized]), \
             patch.object(release, "ensure_tag"), patch.object(release, "release_notes", return_value="Notes"), \
             patch.object(release, "run", side_effect=command), patch.object(release, "verify_remote_assets"), \
             patch.object(release, "should_promote_latest", return_value=False), \
             patch.object(release, "api", return_value={**normalized, "draft": False}), \
             patch.object(release, "record_publication"):
            release.publish("v0.12.3", "a" * 40, self.root, "success", "success")
        self.assertIn("--notes-file", commands[0])
        self.assertIn("--latest=false", commands[-1])
        self.assertTrue(all("create" not in command for command in commands))

    def test_conflicting_draft_attestation_is_rejected_before_upload(self):
        draft = {"tag_name": "v0.12.3", "draft": True, "body": "<!-- cuepool-source: wrong -->"}
        with patch.object(release, "find_release", return_value=draft), patch.object(release, "ensure_tag"), \
             patch.object(release, "release_notes", return_value="Notes"), patch.object(release, "run") as run:
            with self.assertRaisesRegex(ValueError, "conflicting source"):
                release.publish("v0.12.3", "a" * 40, self.root, "success", "success")
            run.assert_not_called()

    def test_notes_use_only_the_requested_version(self):
        (self.root / "CHANGELOG.md").write_text("# Changes\n## [Unreleased]\nfuture\n## [0.12.3]\ncurrent\n## [0.12.2]\nold\n")
        self.assertEqual(release.release_notes("v0.12.3", self.root), "current")


if __name__ == "__main__":
    unittest.main()
