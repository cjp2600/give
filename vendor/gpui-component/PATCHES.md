# Local changes to gpui-component 0.5.1

Source: the published `gpui-component` 0.5.1 crate from crates.io, licensed under
Apache-2.0. The original license is in `LICENSE-APACHE`.

`src/input/selection.rs` classifies Unicode letters and numbers as word characters
using `char::is_alphanumeric`, matching give's source selection. Upstream used
`is_ascii_alphanumeric`, which selected one Cyrillic letter on double-click.
Word selection also applies while dragging after a double-click. The patch covers
all inputs, including the message composer, find fields and commit messages.

Selection tests cover Cyrillic, `ё`, accented Latin, Greek, mixed identifiers and
UTF-8 offsets. CJK letters now form contiguous words as other letters do.

The root `[patch.crates-io]` override keeps this fix reproducible without changing
the Cargo registry cache. Remove it when upgrading to an upstream version with
equivalent Unicode word selection.
