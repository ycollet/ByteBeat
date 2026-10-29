#!/usr/bin/env bash
set -euo pipefail

format="${1:?Usage: package-linux.sh deb|rpm}"
: "${VERSION:?}" "${BINARY_PATH:?}" "${GITHUB_REPOSITORY:?}" "${GITHUB_REPOSITORY_OWNER:?}"
case "$format" in
  deb) : "${DEBIAN_VERSION:?}" ;;
  rpm) : "${BASE_VERSION:?}" "${RPM_RELEASE:?}" ;;
  *) echo "Unknown package format: $format" >&2; exit 1 ;;
esac

mkdir -p target/packaging dist
work="$(mktemp -d "$PWD/target/packaging/$format.XXXXXX")"
root="$work/root"
install -Dm755 "$BINARY_PATH" "$root/usr/bin/bytebeat"
install -Dm644 packaging/bytebeat.desktop "$root/usr/share/applications/bytebeat.desktop"
install -Dm644 assets/bytebeat.svg "$root/usr/share/icons/hicolor/scalable/apps/bytebeat.svg"
install -Dm644 README.md "$root/usr/share/doc/bytebeat/README.md"
install -Dm644 DOCUMENTATION.md "$root/usr/share/doc/bytebeat/DOCUMENTATION.md"
desktop-file-validate "$root/usr/share/applications/bytebeat.desktop"

if [[ "$format" == deb ]]; then
  install -Dm644 LICENSE "$root/usr/share/doc/bytebeat/copyright"
  install -Dm644 assets/VERTEXSHADERART-LICENSE.txt \
    "$root/usr/share/doc/bytebeat/VERTEXSHADERART-LICENSE.txt"
  mkdir -p "$root/DEBIAN" "$work/debian"
  cat > "$work/debian/control" <<EOF
Source: bytebeat
Section: sound
Priority: optional
Maintainer: Bytebeat contributors <${GITHUB_REPOSITORY_OWNER}@users.noreply.github.com>

Package: bytebeat
Architecture: amd64
Description: A standalone native bytebeat composer
EOF
  dependencies="$(cd "$work" && dpkg-shlibdeps -O -e"$root/usr/bin/bytebeat")"
  cat > "$root/DEBIAN/control" <<EOF
Package: bytebeat
Version: $DEBIAN_VERSION
Section: sound
Priority: optional
Architecture: amd64
Maintainer: Bytebeat contributors <${GITHUB_REPOSITORY_OWNER}@users.noreply.github.com>
Homepage: https://github.com/$GITHUB_REPOSITORY
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: ${dependencies#shlibs:Depends=}, libjack-jackd2-0 | libjack-0, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libegl1, libgl1
Description: A standalone native bytebeat composer
 Native JavaScript bytebeat editor with real-time audio, visualizations,
 JACK playback and NSM session management.
EOF
  dpkg-deb --root-owner-group --build "$root" "dist/bytebeat_${DEBIAN_VERSION}_amd64.deb"
else
  install -Dm644 LICENSE "$root/usr/share/licenses/bytebeat/LICENSE"
  install -Dm644 assets/VERTEXSHADERART-LICENSE.txt \
    "$root/usr/share/licenses/bytebeat/VERTEXSHADERART-LICENSE.txt"
  mkdir -p "$work/rpm"
  cat > "$work/bytebeat.spec" <<EOF
Name: bytebeat
Version: $BASE_VERSION
Release: $RPM_RELEASE%{?dist}
Summary: A standalone native bytebeat composer
License: MIT AND CC0-1.0
URL: https://github.com/$GITHUB_REPOSITORY
BuildArch: x86_64
Requires: libjack.so.0()(64bit)
Requires: libX11, libXcursor, libXi, libXrandr, libxkbcommon, libxkbcommon-x11
Requires: libwayland-client, libwayland-cursor, libwayland-egl, libglvnd-egl, libglvnd-glx
%global debug_package %{nil}

%description
Native JavaScript bytebeat editor with real-time audio, visualizations,
JACK playback and NSM session management.

%prep

%build

%install
mkdir -p %{buildroot}
cp -a "$root/usr" %{buildroot}/

%files
/usr/bin/bytebeat
/usr/share/applications/bytebeat.desktop
/usr/share/icons/hicolor/scalable/apps/bytebeat.svg
%doc /usr/share/doc/bytebeat
%license /usr/share/licenses/bytebeat
EOF
  rpmbuild -bb --define "_topdir $work/rpm" "$work/bytebeat.spec"
  cp "$work"/rpm/RPMS/x86_64/*.rpm dist/
fi
