#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { echo 'Run this bundle script on macOS.' >&2; exit 1; }
profile=${PROFILE:-release}
case "$profile" in
    release) cargo build --release ;;
    debug) cargo build ;;
    *) echo 'PROFILE must be debug or release.' >&2; exit 1 ;;
esac
test -f assets/icons/give.icns || ./scripts/build-icons.sh
bundle=dist/give.app
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/give" "$bundle/Contents/MacOS/give"
cp assets/icons/give.icns "$bundle/Contents/Resources/give.icns"
cp assets/Info.plist "$bundle/Contents/Info.plist"
printf 'APPL????' > "$bundle/Contents/PkgInfo"
codesign --force --deep --sign - "$bundle"
echo "Built $bundle ($profile)"
