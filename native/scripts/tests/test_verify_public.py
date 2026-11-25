import json
from pathlib import Path
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
import verify_public


class PublicVerificationTests(unittest.TestCase):
    def test_rejects_incomplete_matrix_before_downloading_archives(self):
        def download(url, path, limit):
            path.write_text(json.dumps(dict(index=json.dumps(dict(schema_version=1, releases=[])), signature="signed")))
        with mock.patch.object(verify_public, "download", side_effect=download) as fetch, mock.patch.object(verify_public, "verify_signature"):
            with self.assertRaisesRegex(ValueError, "incomplete public platform matrix"):
                verify_public.verify(["vnc"], "1.0.0")
            self.assertEqual(fetch.call_count, 1)

    def test_untrusted_index_never_downloads_packages(self):
        def download(url, path, limit):
            path.write_text(json.dumps(dict(index="{}", signature="wrong")))
        with mock.patch.object(verify_public, "download", side_effect=download) as fetch, mock.patch.object(verify_public, "verify_signature", side_effect=ValueError("signature")):
            with self.assertRaises(ValueError): verify_public.verify(["vnc"], "1.0.0")
            self.assertEqual(fetch.call_count, 1)

    def test_package_size_mismatch_rejected_before_manifest_use(self):
        releases = [dict(component="vnc", version="1.0.0", target=t, runtime=r,
                         url="https://downloads.nyaterm.app/native/v1/test.zip", size=100, sha256="a" * 64)
                    for t, r in verify_public.PLATFORMS]
        def download(url, path, limit):
            if url == verify_public.INDEX:
                path.write_text(json.dumps(dict(index=json.dumps(dict(schema_version=1, releases=releases)), signature="signed")))
            else: path.write_bytes(b"truncated")
        with mock.patch.object(verify_public, "download", side_effect=download), mock.patch.object(verify_public, "verify_signature"), mock.patch.object(verify_public, "inspect") as inspect:
            with self.assertRaisesRegex(ValueError, "digest/size mismatch"):
                verify_public.verify(["vnc"], "1.0.0")
            inspect.assert_not_called()

    def test_untrusted_url_is_rejected_without_network(self):
        with mock.patch.object(verify_public, "urlopen") as open_url:
            with self.assertRaises(ValueError):
                verify_public.download("https://example.com/native/v1/a.zip", Path("unused"), 100)
            open_url.assert_not_called()
