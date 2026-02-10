// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, io::Write};

use moss::Dependency;
use stone::{
    StoneHeaderV1FileType, StonePayloadMetaPrimitive, StonePayloadMetaRecord, StonePayloadMetaTag, StoneWriter,
};

use super::Error;
use crate::package::emit::Package;

pub fn write<W: Write>(
    output: &mut W,
    packages: &BTreeSet<&Package<'_>>,
    build_deps: &BTreeSet<String>,
) -> Result<(), Error> {
    let mut writer = StoneWriter::new(output, StoneHeaderV1FileType::BuildManifest)?;

    // Add each package
    for package in packages {
        let mut meta = package.meta();
        // deliberately override .stone package metadata and set build_release to zero for binary manifests
        meta.build_release = 0;
        let mut payload = meta.to_stone_payload();

        // Add build deps
        for name in build_deps {
            if let Ok(dep) = Dependency::from_name(name) {
                payload.push(StonePayloadMetaRecord {
                    tag: StonePayloadMetaTag::BuildDepends,
                    primitive: StonePayloadMetaPrimitive::Dependency(dep.kind.into(), dep.name),
                });
            }
        }

        writer.add_payload(payload.as_slice())?;
    }

    writer.finalize()?;

    Ok(())
}
