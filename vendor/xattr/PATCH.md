# xattr 0.2.3 compatibility patch

Source: the xattr 0.2.3 crate published on crates.io, upstream https://github.com/Stebalien/xattr. The original MIT and Apache 2.0 licenses are retained.

GPUI 0.2.2 pulls this version through gpui_http_client and zed-async-tar. Recent libc releases removed the deprecated ENOATTR alias on Linux and Android. Use ENODATA on those platforms, preserving ENOATTR on macOS and BSD and preserving xattr's public API.

Upstream context: https://github.com/rust-lang/libc/issues/1356

Only src/sys/mod.rs differs from upstream. The upstream tests are included.
