#!/usr/bin/env bash
# Build MacView in release mode and assemble MacView.app next to this repo's
# target/ directory. Run on the Mac:   ./packaging/bundle.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release
APP=target/MacView.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/macview "$APP/Contents/MacOS/macview"
cp packaging/Info.plist "$APP/Contents/Info.plist"
echo -n 'APPL????' > "$APP/Contents/PkgInfo"

# Icon: optional. Drop a 1024×1024 PNG at packaging/icon.png and it becomes MacView.icns.
if [ -f packaging/icon.png ] && command -v iconutil >/dev/null; then
  ICONSET=$(mktemp -d)/MacView.iconset
  mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    sips -z $s $s packaging/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) packaging/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/MacView.icns"
fi

# Ad-hoc signature so Gatekeeper and TCC behave on Apple Silicon.
codesign --force --sign - "$APP" 2>/dev/null || true

echo "Built $APP"
echo "Install:  cp -r $APP /Applications/"
echo "Then right-click any image > Open With > MacView (or set it as default in Get Info)."
