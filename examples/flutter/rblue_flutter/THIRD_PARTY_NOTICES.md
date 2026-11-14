# Third-party notices

This package distributes prebuilt native libraries and Android Java helper
sources. The inventories below were generated from the locked, target-filtered
Rust graphs with `cargo-about 0.9.3`; build-only and development-only crates are
excluded. Source and exact build instructions are in `native/BUILDING.md`.

The rblue project code is licensed `MIT OR Apache-2.0`; see `LICENSE-MIT` and
`LICENSE-APACHE`. The Android Java helpers are vendored from the pinned btleplug
fork and retain their upstream notice in `android/BLEPLUG_JAVA_NOTICE.md`. In
particular, btleplug includes BSD-3-Clause code Copyright (c) 2020-2021
Nonpolynomial, portions from Rumble Copyright (c) 2014 The Rust Project
Developers, and portions from blurmac Copyright (c) 2017 Akos Kiss.

The package chooses a permitted license from each `OR` expression only for the
purpose of satisfying redistribution; recipients retain every option granted by
the upstream expression.

## Android native dependency inventory

| Component | Version | Declared license | Source |
| --- | --- | --- | --- |
| [async-trait](https://github.com/dtolnay/async-trait) | 0.1.92 | `MIT OR Apache-2.0` | https://github.com/dtolnay/async-trait |
| [bitflags](https://github.com/bitflags/bitflags) | 2.13.2 | `MIT OR Apache-2.0` | https://github.com/bitflags/bitflags |
| [btleplug](https://github.com/deviceplug/btleplug) | 0.13.5 | `MIT OR Apache-2.0 OR BSD-3-Clause` | https://github.com/deviceplug/btleplug |
| [bytes](https://github.com/tokio-rs/bytes) | 1.12.1 | `MIT` | https://github.com/tokio-rs/bytes |
| [cfg-if](https://github.com/rust-lang/cfg-if) | 1.0.5 | `MIT OR Apache-2.0` | https://github.com/rust-lang/cfg-if |
| [combine](https://github.com/Marwes/combine) | 4.6.9 | `MIT` | https://github.com/Marwes/combine |
| [crossbeam-utils](https://github.com/crossbeam-rs/crossbeam) | 0.8.23 | `MIT OR Apache-2.0` | https://github.com/crossbeam-rs/crossbeam |
| [dashmap](https://github.com/xacrimon/dashmap) | 6.2.1 | `MIT` | https://github.com/xacrimon/dashmap |
| [futures](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-channel](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-core](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-executor](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-io](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-macro](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-sink](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-task](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-timer](https://github.com/async-rs/futures-timer) | 3.0.4 | `MIT OR Apache-2.0` | https://github.com/async-rs/futures-timer |
| [futures-util](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [getrandom](https://github.com/rust-random/getrandom) | 0.4.3 | `MIT OR Apache-2.0` | https://github.com/rust-random/getrandom |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.14.5 | `MIT OR Apache-2.0` | https://github.com/rust-lang/hashbrown |
| [itoa](https://github.com/dtolnay/itoa) | 1.0.18 | `MIT OR Apache-2.0` | https://github.com/dtolnay/itoa |
| [jni](https://github.com/jni-rs/jni-rs) | 0.22.4 | `MIT OR Apache-2.0` | https://github.com/jni-rs/jni-rs |
| [jni-macros](https://github.com/jni-rs/jni-rs) | 0.22.4 | `MIT OR Apache-2.0` | https://github.com/jni-rs/jni-rs |
| [jni-sys](https://github.com/jni-rs/jni-sys) | 0.4.1 | `MIT OR Apache-2.0` | https://github.com/jni-rs/jni-sys |
| [jni-sys-macros](https://github.com/jni-rs/jni-sys) | 0.4.1 | `MIT OR Apache-2.0` | https://github.com/jni-rs/jni-sys |
| [libc](https://github.com/rust-lang/libc) | 0.2.191 | `MIT OR Apache-2.0` | https://github.com/rust-lang/libc |
| [lock_api](https://github.com/Amanieu/parking_lot) | 0.4.14 | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| [log](https://github.com/rust-lang/log) | 0.4.35 | `MIT OR Apache-2.0` | https://github.com/rust-lang/log |
| [memchr](https://github.com/BurntSushi/memchr) | 2.8.3 | `Unlicense OR MIT` | https://github.com/BurntSushi/memchr |
| [once_cell](https://github.com/matklad/once_cell) | 1.21.4 | `MIT OR Apache-2.0` | https://github.com/matklad/once_cell |
| [parking_lot_core](https://github.com/Amanieu/parking_lot) | 0.9.12 | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| [pin-project-lite](https://github.com/taiki-e/pin-project-lite) | 0.2.17 | `Apache-2.0 OR MIT` | https://github.com/taiki-e/pin-project-lite |
| [proc-macro2](https://github.com/dtolnay/proc-macro2) | 1.0.107 | `MIT OR Apache-2.0` | https://github.com/dtolnay/proc-macro2 |
| [quote](https://github.com/dtolnay/quote) | 1.0.47 | `MIT OR Apache-2.0` | https://github.com/dtolnay/quote |
| [rblue](https://github.com/rblue-rs/rblue) | 0.0.1 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-android](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-api](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-c](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [scopeguard](https://github.com/bluss/scopeguard) | 1.2.0 | `MIT OR Apache-2.0` | https://github.com/bluss/scopeguard |
| [serde](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_core](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_derive](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_json](https://github.com/serde-rs/json) | 1.0.154 | `MIT OR Apache-2.0` | https://github.com/serde-rs/json |
| [simd_cesu8](https://github.com/seancroach/simd_cesu8) | 1.2.0 | `Apache-2.0 OR MIT` | https://github.com/seancroach/simd_cesu8 |
| [simdutf8](https://github.com/rusticstuff/simdutf8) | 0.1.5 | `MIT OR Apache-2.0` | https://github.com/rusticstuff/simdutf8 |
| [slab](https://github.com/tokio-rs/slab) | 0.4.12 | `MIT` | https://github.com/tokio-rs/slab |
| [smallvec](https://github.com/servo/rust-smallvec) | 1.16.5 | `MIT OR Apache-2.0` | https://github.com/servo/rust-smallvec |
| [static_assertions](https://github.com/nvzqz/static-assertions-rs) | 1.1.0 | `MIT OR Apache-2.0` | https://github.com/nvzqz/static-assertions-rs |
| [syn](https://github.com/dtolnay/syn) | 2.0.119 | `MIT OR Apache-2.0` | https://github.com/dtolnay/syn |
| [syn](https://github.com/dtolnay/syn) | 3.0.9 | `MIT OR Apache-2.0` | https://github.com/dtolnay/syn |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.21 | `MIT OR Apache-2.0` | https://github.com/dtolnay/thiserror |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 2.0.21 | `MIT OR Apache-2.0` | https://github.com/dtolnay/thiserror |
| [tokio](https://github.com/tokio-rs/tokio) | 1.54.1 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-macros](https://github.com/tokio-rs/tokio) | 2.7.2 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-stream](https://github.com/tokio-rs/tokio) | 0.1.19 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-util](https://github.com/tokio-rs/tokio) | 0.7.20 | `MIT` | https://github.com/tokio-rs/tokio |
| [tracing](https://github.com/tokio-rs/tracing) | 0.1.44 | `MIT` | https://github.com/tokio-rs/tracing |
| [tracing-attributes](https://github.com/tokio-rs/tracing) | 0.1.31 | `MIT` | https://github.com/tokio-rs/tracing |
| [tracing-core](https://github.com/tokio-rs/tracing) | 0.1.36 | `MIT` | https://github.com/tokio-rs/tracing |
| [unicode-ident](https://github.com/dtolnay/unicode-ident) | 1.0.27 | `(MIT OR Apache-2.0) AND Unicode-3.0` | https://github.com/dtolnay/unicode-ident |
| [uuid](https://github.com/uuid-rs/uuid) | 1.29.1 | `Apache-2.0 OR MIT` | https://github.com/uuid-rs/uuid |
| [zmij](https://github.com/dtolnay/zmij) | 1.0.23 | `MIT` | https://github.com/dtolnay/zmij |

## iOS native dependency inventory

| Component | Version | Declared license | Source |
| --- | --- | --- | --- |
| [async-trait](https://github.com/dtolnay/async-trait) | 0.1.92 | `MIT OR Apache-2.0` | https://github.com/dtolnay/async-trait |
| [bitflags](https://github.com/bitflags/bitflags) | 2.13.2 | `MIT OR Apache-2.0` | https://github.com/bitflags/bitflags |
| [block2](https://github.com/madsmtm/objc2) | 0.6.2 | `MIT` | https://github.com/madsmtm/objc2 |
| [btleplug](https://github.com/deviceplug/btleplug) | 0.13.5 | `MIT OR Apache-2.0 OR BSD-3-Clause` | https://github.com/deviceplug/btleplug |
| [bytes](https://github.com/tokio-rs/bytes) | 1.12.1 | `MIT` | https://github.com/tokio-rs/bytes |
| [cfg-if](https://github.com/rust-lang/cfg-if) | 1.0.5 | `MIT OR Apache-2.0` | https://github.com/rust-lang/cfg-if |
| [crossbeam-utils](https://github.com/crossbeam-rs/crossbeam) | 0.8.23 | `MIT OR Apache-2.0` | https://github.com/crossbeam-rs/crossbeam |
| [dashmap](https://github.com/xacrimon/dashmap) | 6.2.1 | `MIT` | https://github.com/xacrimon/dashmap |
| [futures](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-channel](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-core](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-executor](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-io](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-macro](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-sink](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-task](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [futures-timer](https://github.com/async-rs/futures-timer) | 3.0.4 | `MIT OR Apache-2.0` | https://github.com/async-rs/futures-timer |
| [futures-util](https://github.com/rust-lang/futures-rs) | 0.3.34 | `MIT OR Apache-2.0` | https://github.com/rust-lang/futures-rs |
| [getrandom](https://github.com/rust-random/getrandom) | 0.4.3 | `MIT OR Apache-2.0` | https://github.com/rust-random/getrandom |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.14.5 | `MIT OR Apache-2.0` | https://github.com/rust-lang/hashbrown |
| [itoa](https://github.com/dtolnay/itoa) | 1.0.18 | `MIT OR Apache-2.0` | https://github.com/dtolnay/itoa |
| [libc](https://github.com/rust-lang/libc) | 0.2.191 | `MIT OR Apache-2.0` | https://github.com/rust-lang/libc |
| [lock_api](https://github.com/Amanieu/parking_lot) | 0.4.14 | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| [log](https://github.com/rust-lang/log) | 0.4.35 | `MIT OR Apache-2.0` | https://github.com/rust-lang/log |
| [memchr](https://github.com/BurntSushi/memchr) | 2.8.3 | `Unlicense OR MIT` | https://github.com/BurntSushi/memchr |
| [objc2](https://github.com/madsmtm/objc2) | 0.6.5 | `MIT` | https://github.com/madsmtm/objc2 |
| [objc2-core-bluetooth](https://github.com/madsmtm/objc2) | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | https://github.com/madsmtm/objc2 |
| [objc2-encode](https://github.com/madsmtm/objc2) | 4.1.0 | `MIT` | https://github.com/madsmtm/objc2 |
| [objc2-foundation](https://github.com/madsmtm/objc2) | 0.3.2 | `MIT` | https://github.com/madsmtm/objc2 |
| [once_cell](https://github.com/matklad/once_cell) | 1.21.4 | `MIT OR Apache-2.0` | https://github.com/matklad/once_cell |
| [parking_lot_core](https://github.com/Amanieu/parking_lot) | 0.9.12 | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| [pin-project-lite](https://github.com/taiki-e/pin-project-lite) | 0.2.17 | `Apache-2.0 OR MIT` | https://github.com/taiki-e/pin-project-lite |
| [proc-macro2](https://github.com/dtolnay/proc-macro2) | 1.0.107 | `MIT OR Apache-2.0` | https://github.com/dtolnay/proc-macro2 |
| [quote](https://github.com/dtolnay/quote) | 1.0.47 | `MIT OR Apache-2.0` | https://github.com/dtolnay/quote |
| [rblue](https://github.com/rblue-rs/rblue) | 0.0.1 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-api](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-c](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [rblue-corebluetooth](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [scopeguard](https://github.com/bluss/scopeguard) | 1.2.0 | `MIT OR Apache-2.0` | https://github.com/bluss/scopeguard |
| [serde](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_core](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_derive](https://github.com/serde-rs/serde) | 1.0.229 | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| [serde_json](https://github.com/serde-rs/json) | 1.0.154 | `MIT OR Apache-2.0` | https://github.com/serde-rs/json |
| [slab](https://github.com/tokio-rs/slab) | 0.4.12 | `MIT` | https://github.com/tokio-rs/slab |
| [smallvec](https://github.com/servo/rust-smallvec) | 1.16.5 | `MIT OR Apache-2.0` | https://github.com/servo/rust-smallvec |
| [static_assertions](https://github.com/nvzqz/static-assertions-rs) | 1.1.0 | `MIT OR Apache-2.0` | https://github.com/nvzqz/static-assertions-rs |
| [syn](https://github.com/dtolnay/syn) | 2.0.119 | `MIT OR Apache-2.0` | https://github.com/dtolnay/syn |
| [syn](https://github.com/dtolnay/syn) | 3.0.9 | `MIT OR Apache-2.0` | https://github.com/dtolnay/syn |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.21 | `MIT OR Apache-2.0` | https://github.com/dtolnay/thiserror |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 2.0.21 | `MIT OR Apache-2.0` | https://github.com/dtolnay/thiserror |
| [tokio](https://github.com/tokio-rs/tokio) | 1.54.1 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-macros](https://github.com/tokio-rs/tokio) | 2.7.2 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-stream](https://github.com/tokio-rs/tokio) | 0.1.19 | `MIT` | https://github.com/tokio-rs/tokio |
| [tokio-util](https://github.com/tokio-rs/tokio) | 0.7.20 | `MIT` | https://github.com/tokio-rs/tokio |
| [tracing](https://github.com/tokio-rs/tracing) | 0.1.44 | `MIT` | https://github.com/tokio-rs/tracing |
| [tracing-attributes](https://github.com/tokio-rs/tracing) | 0.1.31 | `MIT` | https://github.com/tokio-rs/tracing |
| [tracing-core](https://github.com/tokio-rs/tracing) | 0.1.36 | `MIT` | https://github.com/tokio-rs/tracing |
| [unicode-ident](https://github.com/dtolnay/unicode-ident) | 1.0.27 | `(MIT OR Apache-2.0) AND Unicode-3.0` | https://github.com/dtolnay/unicode-ident |
| [uuid](https://github.com/uuid-rs/uuid) | 1.29.1 | `Apache-2.0 OR MIT` | https://github.com/uuid-rs/uuid |
| [zmij](https://github.com/dtolnay/zmij) | 1.0.23 | `MIT` | https://github.com/dtolnay/zmij |

## macOS native dependency inventory

The macOS universal archive uses the same locked CoreBluetooth dependency graph
listed in the iOS inventory above. Architecture does not change the dependency
or license set.

## Windows native dependency inventory

The Windows DLL shares the common Rust dependencies listed above and adds the
following WinRT-specific components:

| Component | Version | Declared license | Source |
| --- | --- | --- | --- |
| [rblue-winrt](https://github.com/rblue-rs/rblue) | 0.1.0 | `MIT OR Apache-2.0` | https://github.com/rblue-rs/rblue |
| [windows](https://github.com/microsoft/windows-rs) | 0.62.2 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-collections](https://github.com/microsoft/windows-rs) | 0.3.2 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.62.2 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-future](https://github.com/microsoft/windows-rs) | 0.3.2 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-implement](https://github.com/microsoft/windows-rs) | 0.60.2 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-interface](https://github.com/microsoft/windows-rs) | 0.59.3 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-link](https://github.com/microsoft/windows-rs) | 0.2.1 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-numerics](https://github.com/microsoft/windows-rs) | 0.3.1 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.4.1 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-strings](https://github.com/microsoft/windows-rs) | 0.5.1 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| [windows-threading](https://github.com/microsoft/windows-rs) | 0.2.1 | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |

## License texts

The complete Apache-2.0 and MIT texts are distributed as `LICENSE-APACHE` and `LICENSE-MIT`. The btleplug/Rumble/blurmac notices and BSD-3-Clause text are in `android/BLEPLUG_JAVA_NOTICE.md`. Additional license text required by the locked native graph follows.

### Unicode License v3

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```
