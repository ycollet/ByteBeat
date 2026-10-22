import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "download_catalog", Path(__file__).with_name("download_catalog.py")
)
downloader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(downloader)


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "songs").mkdir()
        self.song = {
            "hash": "a" * 32, "name": "Test", "sampleRate": 8000,
            "code": "t", "author": "Original author",
        }

    def test_inline_source_and_attribution(self):
        result = downloader.download_song(self.root, "Group author", self.song)
        self.assertEqual((self.root / result["file"]).read_text(), "t")
        self.assertEqual(result["author"], "Original author")
        self.assertEqual(result["upstream"], self.song)
        sidecar = json.loads((self.root / result["file"]).with_suffix(".json").read_text())
        self.assertEqual(sidecar["sha256"], result["sha256"])

    def test_fetches_full_original_not_preview_and_reuses_verified_cache(self):
        del self.song["code"]
        self.song.update(fileOrig=1, codeText="incomplete preview")
        with patch.object(downloader, "fetch", return_value=b"t * 2;") as fetch:
            first = downloader.download_song(self.root, "Author", self.song)
            second = downloader.download_song(self.root, "Author", self.song)
        fetch.assert_called_once_with(
            downloader.BASE + "data/songs/original/" + "a" * 32 + ".js"
        )
        self.assertEqual(first, second)
        self.assertEqual((self.root / first["file"]).read_text(), "t * 2;")

    def test_missing_source_and_invalid_ids_are_not_silently_accepted(self):
        self.song.pop("code")
        self.song["codeText"] = "only a preview"
        with self.assertRaises(ValueError):
            downloader.download_song(self.root, "Author", self.song)
        self.song["hash"] = "../not-a-song"
        with self.assertRaises(ValueError):
            downloader.download_song(self.root, "Author", self.song)

    def test_modified_sources_and_forced_refresh_invalidate_cache(self):
        del self.song["code"]
        self.song["fileOrig"] = 1
        with patch.object(downloader, "fetch", return_value=b"t") as fetch:
            first = downloader.download_song(self.root, "Author", self.song)
            (self.root / first["file"]).write_text("corrupted")
            downloader.download_song(self.root, "Author", self.song)
            downloader.download_song(self.root, "Author", self.song, refresh=True)
            self.song["date_edited"] = "2026-10-22"
            downloader.download_song(self.root, "Author", self.song)
        self.assertEqual(fetch.call_count, 4)


if __name__ == "__main__":
    unittest.main()
