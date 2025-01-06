// SPDX-FileCopyrightText: Copyright © 2020-2024 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeSet, path::Path};

use fs_err::File;
use moss::Dependency;
use stone::{StoneHeaderV1FileType, StonePayloadMetaBody, StonePayloadMetaKind, StonePayloadMetaTag, StoneWriter};

use super::Error;
use crate::package::emit::Package;

pub fn write(path: &Path, packages: &BTreeSet<&Package<'_>>, build_deps: &BTreeSet<String>) -> Result<(), Error> {
    let mut output = File::create(path)?;

    let mut writer = StoneWriter::new(&mut output, StoneHeaderV1FileType::BuildManifest)?;

    // Add each package
    for package in packages {
        let mut meta = package.meta();
        // deliberately override .stone package metadata and set build_release to zero for binary manifests
        meta.build_release = 0;
        let mut payload = meta.to_stone_payload();

        // Add build deps
        for name in build_deps {
            if let Ok(dep) = Dependency::from_name(name) {
                payload.push(StonePayloadMetaBody {
                    tag: StonePayloadMetaTag::BuildDepends,
                    kind: StonePayloadMetaKind::Dependency(dep.kind.into(), dep.name),
                });
            }
        }

        writer.add_payload(payload.as_slice())?;
    }

    writer.finalize()?;

    Ok(())
}
