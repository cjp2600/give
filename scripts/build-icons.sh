#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    echo 'Icon export uses macOS Swift/AppKit and iconutil. Exported PNG/ICO files work on all platforms.' >&2
    exit 1
fi

icons=assets/icons
swift scripts/render-icons.swift "$icons"
for variant in default light dark; do
    case "$variant" in
        default) source="$icons/give-dark.png"; stem=give ;;
        *) source="$icons/give-$variant.png"; stem="give-$variant" ;;
    esac
    test -f "$source" || { echo "Missing artwork: $source" >&2; exit 1; }
    pngs="$icons/png/$variant"
    iconset="$icons/$stem.iconset"
    mkdir -p "$pngs" "$iconset"
    for logical in 16 32 128 256 512; do
        retina=$((logical * 2))
        cp "$pngs/$logical.png" "$iconset/icon_${logical}x${logical}.png"
        cp "$pngs/$retina.png" "$iconset/icon_${logical}x${logical}@2x.png"
    done
    iconutil -c icns "$iconset" -o "$icons/$stem.icns"
    python3 scripts/pack-ico.py "$pngs" "$icons/$stem.ico"
    echo "Exported $variant: PNG 16–1024, macOS ICNS, Windows ICO"
done
