#!/usr/bin/env bash
set -euo pipefail
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app="$stage/PMQuant Terminal.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp app/assets/AppIcon.icns "$app/Contents/Resources/"
lipo -create target/{aarch64-apple-darwin,x86_64-apple-darwin}/release/terminal-rs -output "$app/Contents/MacOS/PMQuantTerminal"
lipo "$app/Contents/MacOS/PMQuantTerminal" -verify_arch arm64 x86_64
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>PMQuant Terminal</string>
<key>CFBundleIdentifier</key><string>app.pmquant.terminal</string>
<key>CFBundleExecutable</key><string>PMQuantTerminal</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.1</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
hdiutil create -ov -volname "PMQuant Terminal" -srcfolder "$stage" -format UDZO PMQuant-Terminal.dmg
hdiutil verify PMQuant-Terminal.dmg
