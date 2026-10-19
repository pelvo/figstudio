#!/usr/bin/env bash
# Builds the desktop app as a macOS bundle, so it starts from Finder, Launchpad or the Dock
# instead of a terminal. The app hosts the MCP server itself (View menu); the bundle also
# carries the standalone figstudio-mcp and figstudio-cli next to it in Contents/MacOS.
#
#   scripts/macos-app.sh              build target/release/figstudio.app
#   scripts/macos-app.sh --install    then copy it to ~/Applications
#
# Needs: macOS with the Xcode command line tools. Uses the same toolchain as check.sh
# (FIGSTUDIO_TOOLCHAIN overrides it). The bundle is ad-hoc signed, so it runs on this Mac
# without a developer account; it is not notarized for other machines.
set -euo pipefail
cd "$(dirname "$0")/.."

[ "$(uname -s)" = Darwin ] || { echo "macos-app.sh: macOS only" >&2; exit 1; }

TC=()
[ -n "${FIGSTUDIO_TOOLCHAIN:-}" ] && TC=("+$FIGSTUDIO_TOOLCHAIN")
cargo "${TC[@]}" build --release -p figstudio -p figstudio-mcp -p figstudio-cli

version=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
app=target/release/figstudio.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/figstudio target/release/figstudio-mcp target/release/figstudio-cli \
  "$app/Contents/MacOS/"

cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>figstudio</string>
  <key>CFBundleDisplayName</key><string>figstudio</string>
  <key>CFBundleIdentifier</key><string>dev.figstudio.app</string>
  <key>CFBundleExecutable</key><string>figstudio</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
EOF

plutil -lint "$app/Contents/Info.plist" >/dev/null
# Helpers first, then the bundle (its seal covers the signed helpers).
codesign --force --sign - "$app/Contents/MacOS/figstudio-mcp" "$app/Contents/MacOS/figstudio-cli"
codesign --force --sign - "$app"
echo "built $app"

if [ "${1:-}" = --install ]; then
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/figstudio.app"
  cp -R "$app" "$HOME/Applications/"
  echo "installed $HOME/Applications/figstudio.app"
fi
