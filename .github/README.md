# Release procedure

The [Release installers workflow](workflows/release.yml) builds and verifies
installers, then publishes a GitHub Release when a version tag is pushed.
Run the commands below from the repository root.

## 1. Update the version

Start from the branch and commit you intend to release, with a clean working
tree. Update `[package].version` in `Cargo.toml`, for example:

```toml
[package]
version = "0.2.0"
```

Refresh the root package version in `Cargo.lock` and check the build:

```sh
cargo check
cargo test --locked
python3 -m unittest discover -s scripts -p test_release_metadata.py
```

Review the diff and commit both version files:

```sh
git diff -- Cargo.toml Cargo.lock
git add Cargo.toml Cargo.lock
git commit -m "Release 0.2.0"
git push origin HEAD
```

If changes require a pull request, merge it first and check out the resulting
release commit before continuing. The tag must point to the commit containing
the updated version files and all intended release changes.

## 2. Create and push the tag

Use `vVERSION` (recommended) or `VERSION`. After removing the optional `v`,
the tag must exactly match `[package].version` in `Cargo.toml`.

Optionally validate the tag locally, then create and push it:

```sh
GITHUB_REF_NAME=v0.2.0 python3 scripts/release_metadata.py
git tag -a v0.2.0 -m "Bytebeat 0.2.0"
git push origin v0.2.0
```

For a prerelease, set the Cargo version to, for example, `0.2.0-rc.1` and
use the tag `v0.2.0-rc.1`. The workflow marks the GitHub Release as a
prerelease. SemVer build metadata such as `0.2.0+build.1` is not supported.

## 3. Monitor and download the release

Open **Actions > Release installers** and select the tag-triggered run.
The workflow runs tests, builds installers, and verifies installation for:

| Platform | Installer | Architecture |
| --- | --- | --- |
| Debian 12+ / compatible Ubuntu releases | `.deb` | x86-64 |
| Fedora 43+ | `.rpm` | x86-64 |
| macOS | `.dmg` | Apple Silicon and Intel |
| Windows 10+ | `.exe` | x86-64 |

Only after every platform succeeds does the publishing job upload the
installers and `SHA256SUMS` and make the GitHub Release public. Release
notes are generated automatically. Find the downloads on the repository's
**Releases** page; installers are also uploaded as workflow artifacts.

No signing secrets are required. Windows installers are unsigned; macOS
apps are ad-hoc signed, not Developer ID-signed or notarized. Users may
see operating-system security warnings.

## Manual installer builds (no release)

Open **Actions > Release installers > Run workflow**, select a branch or
tag, and run it. Download installers from the completed run's **Artifacts**
section. Manual runs use the selected commit's Cargo version and never
create or update a GitHub Release, even when a tag is selected.

The workflow must be present on the default branch for **Run workflow**
to appear.

## Failed releases

Inspect the failed job's logs. For a transient failure, rerun the failed
jobs from the Actions run. Publishing reruns replace existing assets for
the same tag; a failed publishing job may leave a draft release.

If a source or workflow change is needed, commit the fix and release a new
version with a new tag rather than moving an existing release tag.
