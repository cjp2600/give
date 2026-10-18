# give icon family

The mark is a single flat blue hexagon. There are no facial features, strokes, gradients, shadows, reflections, textures, or small details.

## Vector masters

- `give-master.svg`: sky-blue mark (`#8CB5D6`) on a transparent background, used inside the app.
- `give-dark.svg`: the same mark on a rounded midnight-blue tile (`#172635`), used for the default application icon.
- `give-light.svg`: the same geometry in blue (`#355C7B`) on a pale blue tile (`#E7EEF5`).

The SVG masters are the source of truth. One large geometric primitive keeps the silhouette readable at small sizes. All three variants use the same coordinates and proportions.

## Exports

Run `make icons` on macOS. `scripts/render-icons.swift` rasterizes the flat SVG geometry directly at every output size using Core Graphics in sRGB; `iconutil` and `scripts/pack-ico.py` package platform containers. No external image tools or generated illustration are required.

- `give-master.png`: 1024 px transparent mark embedded in the app.
- `give-light.png`, `give-dark.png`: 1024 px appearance variants.
- `give.icns`, `give-light.icns`, `give-dark.icns`: macOS containers, all standard 1×/2× sizes from 16 to 1024 pixels. The default uses the dark tile.
- Corresponding `.iconset/` folders: individual macOS sizes.
- `give.ico`, `give-light.ico`, `give-dark.ico`: Windows containers, 16/24/32/48/64/128/256 pixels.
- `png/{default,light,dark}/`: 16/24/32/48/64/128/256/512/1024 px PNGs, including Linux launcher sizes.

The macOS bundle uses `give.icns`. Light/dark are supplied assets; automatic system appearance switching of the app icon is not implemented.
