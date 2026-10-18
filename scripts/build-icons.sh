#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    echo 'Icon export uses macOS sips and iconutil. Exported PNG/ICO files work on all platforms.' >&2
    exit 1
fi

icons=assets/icons
for variant in default light dark; do
    case "$variant" in
        default) source="$icons/give-master.png"; stem=give ;;
        *) source="$icons/give-$variant.png"; stem="give-$variant" ;;
    esac
    test -f "$source" || { echo "Missing artwork: $source" >&2; exit 1; }
    pngs="$icons/png/$variant"
    iconset="$icons/$stem.iconset"
    mkdir -p "$pngs" "$iconset"
    for pixels in 16 24 32 48 64 128 256 512 1024; do
        sips -s format png -z "$pixels" "$pixels" "$source" --out "$pngs/$pixels.png" >/dev/null
    done
    for logical in 16 32 128 256 512; do
        retina=$((logical * 2))
        cp "$pngs/$logical.png" "$iconset/icon_${logical}x${logical}.png"
        cp "$pngs/$retina.png" "$iconset/icon_${logical}x${logical}@2x.png"
    done
    iconutil -c icns "$iconset" -o "$icons/$stem.icns"
    python3 scripts/pack-ico.py "$pngs" "$icons/$stem.ico"
    echo "Exported $variant: PNG 16–1024, macOS ICNS, Windows ICO"
done
