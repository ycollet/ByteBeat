#!/usr/bin/env bash
set -euo pipefail

: "${VERSION:?}" "${BASE_VERSION:?}" "${TARGET:?}" "${MACOSX_DEPLOYMENT_TARGET:?}"
mkdir -p target/packaging dist
work="$(mktemp -d "$PWD/target/packaging/macos.XXXXXX")"
app="$work/image/Bytebeat Composer.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$TARGET/release/bytebeat" "$app/Contents/MacOS/bytebeat"
cp LICENSE README.md DOCUMENTATION.md assets/VERTEXSHADERART-LICENSE.txt "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>bytebeat</string>
  <key>CFBundleName</key><string>Bytebeat Composer</string>
  <key>CFBundleDisplayName</key><string>Bytebeat Composer</string>
  <key>CFBundleIdentifier</key><string>io.github.bytebeat.composer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$BASE_VERSION</string>
  <key>CFBundleVersion</key><string>$BASE_VERSION</string>
  <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
"$app/Contents/MacOS/bytebeat" --version
ln -s /Applications "$work/image/Applications"
image="dist/bytebeat-$VERSION-$TARGET.dmg"
hdiutil create -volname "Bytebeat Composer" -srcfolder "$work/image" -ov -format UDZO "$image"
hdiutil verify "$image"
