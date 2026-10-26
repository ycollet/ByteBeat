import unittest

from release_metadata import metadata


class ReleaseMetadataTests(unittest.TestCase):
    def test_stable_versions_with_or_without_v_prefix(self):
        expected = {
            "version": "1.2.3",
            "base_version": "1.2.3",
            "debian_version": "1.2.3",
            "rpm_release": "1",
            "prerelease": "false",
        }
        for tag in ["v1.2.3", "1.2.3"]:
            self.assertEqual(metadata(tag, "1.2.3"), expected)

    def test_prereleases_use_native_package_ordering(self):
        result = metadata("v1.2.3-rc.1", "1.2.3-rc.1")
        self.assertEqual(result["base_version"], "1.2.3")
        self.assertEqual(result["debian_version"], "1.2.3~rc.1")
        self.assertEqual(result["rpm_release"], "0.rc.1")
        self.assertEqual(result["prerelease"], "true")
        self.assertEqual(
            metadata("v1.2.3-beta-1", "1.2.3-beta-1")["rpm_release"], "0.beta.1"
        )

    def test_version_mismatch_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "does not match"):
            metadata("v1.2.3", "0.1.0")

    def test_invalid_tags_cannot_become_package_metadata(self):
        for version in [
            "latest", "1.2", "01.2.3", "1.2.3+build", "1.2.3-01",
            "1.2.3-rc..1", "1.2.3\ninjected=value", "1.2.3;echo bad",
        ]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                metadata(version, version)


if __name__ == "__main__":
    unittest.main()
