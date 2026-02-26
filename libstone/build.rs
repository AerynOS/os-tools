// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let bindings = cbindgen::generate(env!("CARGO_MANIFEST_DIR")).unwrap();

    bindings.write_to_file(root.join("src/stone.h"));
}
