import base64
import json
from pathlib import Path
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
import components
import preflight


class PreflightTests(unittest.TestCase):
    def config(self):
        return dict(GITHUB_REF="refs/heads/main", R2_ACCOUNT_ID="a" * 32,
                    R2_ACCESS_KEY_ID="fixture", R2_SECRET_ACCESS_KEY="fixture",
                    TAURI_SIGNING_PRIVATE_KEY_B64=base64.b64encode(b"fixture-key").decode(),
                    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="fixture-password")

    def test_publication_requires_all_protected_secrets_and_main(self):
        env = self.config()
        preflight.configuration(env)
        for key in env:
            with self.subTest(key=key), self.assertRaises(ValueError):
                preflight.configuration({k: v for k, v in env.items() if k != key})
        with self.assertRaises(ValueError):
            preflight.configuration(dict(env, GITHUB_REF="refs/heads/feature"))

    def test_invalid_key_encoding_is_redacted(self):
        with self.assertRaisesRegex(ValueError, "Invalid signing key encoding"):
            preflight.configuration(dict(self.config(), TAURI_SIGNING_PRIVATE_KEY_B64="bad secret!"))

    def test_cli_without_conditional_writes_is_rejected_without_network_io(self):
        with mock.patch.object(preflight.subprocess, "run", return_value=mock.Mock(stdout="{}")):
            with self.assertRaises(ValueError): preflight.tools()
        with mock.patch.object(preflight.subprocess, "run", return_value=mock.Mock(stdout=json.dumps(dict(IfMatch="", IfNoneMatch="")))) as run:
            preflight.tools()
            self.assertIn("--generate-cli-skeleton", run.call_args.args[0])

    def test_wrong_signing_key_is_rejected(self):
        with mock.patch.object(components, "sign_bytes", return_value="wrong-key-signature"), mock.patch.object(components, "verify_signature", side_effect=ValueError("invalid")):
            with self.assertRaises(ValueError): preflight.signing()

    def test_key_decoded_only_into_child_environment(self):
        def signer(command, **kwargs):
            self.assertEqual(kwargs["env"]["TAURI_SIGNING_PRIVATE_KEY"], "fixture-key")
            self.assertNotIn("fixture-password", command)
            self.assertNotIn("fixture-key", command)
            self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY_B64", kwargs["env"])
            Path(command[-1]).with_suffix(".sig").write_bytes(base64.b64encode(b"signature"))
            return mock.Mock(returncode=0)
        with mock.patch.dict(components.os.environ, self.config(), clear=True):
            with mock.patch.object(components.subprocess, "run", side_effect=signer):
                self.assertEqual(components.sign_bytes(b"message"), "signature")
            self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY", components.os.environ)
