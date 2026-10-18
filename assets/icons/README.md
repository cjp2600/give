# give icon family

The mark is a cartoon welding helmet: an orange shell, dark frame and blue glass. Artwork was generated using the **built-in imagegen tool**, with a transparent background. The original master is kept at its generated resolution; platform exports are normalized from it using macOS `sips`.

## Source artwork

- `give-master.png`: the isolated mark, transparent alpha; used in the app and as its default Dock icon.
- `give-light.png`: the transparent master reused for light backgrounds; no opaque tile.
- `give-dark.png`: matching mark on a graphite rounded tile.

## Exports

Run `make icons` on macOS to regenerate all formats:

- `give.icns`, `give-light.icns`, `give-dark.icns`: macOS containers, all standard 1×/2× sizes from 16 to 1024 pixels.
- The corresponding `.iconset/` folders: editable individual macOS sizes.
- `give.ico`, `give-light.ico`, `give-dark.ico`: Windows containers, 16/24/32/48/64/128/256 pixels.
- `png/{default,light,dark}/`: 16/24/32/48/64/128/256/512/1024 px PNGs, including Linux app launcher sizes.

The default macOS bundle uses `give.icns`. Light/dark are supplied assets; automatic system appearance switching of the app icon is not implemented.

## Generation prompts

Master:

> Use case: logo-brand. Asset type: production desktop application icon master for an app named give. Primary request: one cartoon welding helmet / welder's protective face mask. A friendly, beautifully designed industrial cartoon object, bold rounded silhouette, large dark glass rectangular visor, softly rounded robust shell, two simple side hinge details, playful but polished desktop-app quality. Helmet only, no human or body. Front view with a very slight three-quarter turn. Generous clear padding, centered single subject, fills around 78% of square canvas. Clean chunky shapes and restrained softly shaded cartoon illustration, readable at 32 pixels. Distinctive warm shell with cool glass reflections, strong visual contrast; avoid busy tiny detail. Background: genuine transparent alpha, no tile, no scenery, no checkerboard. No lettering, no words, no numbers, no watermark, no extra tools or sparks. Output a single square icon, not a contact sheet or mockup.

Dark (an edit of the master; light reuses the original transparent artwork):

> Edit target: the supplied orange cartoon welding helmet icon. Use case: logo-brand. Create the dark appearance application icon for desktop app give. Preserve the helmet identity, shape, orange shell, blue-black visor and rounded cartoon rendering. Center the complete unchanged helmet inside a softly rounded square deep graphite tile with subtle depth. Helmet should occupy 76% of the canvas leaving clean margins. Genuine transparent alpha outside tile rounded corners. Single square icon only. No text, no additional objects, no checkerboard or mockup.
