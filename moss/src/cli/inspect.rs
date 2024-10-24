// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use clap::{ArgMatches, Command, arg};
use fs_err::File;
use std::io::{Read, Seek, sink};
use std::path::PathBuf;
use stone::{
    StoneDecodedPayload, StonePayloadLayoutFile, StonePayloadMetaPrimitive, StonePayloadMetaTag, StoneReadError,
};
use thiserror::Error;

const COLUMN_WIDTH: usize = 20;

pub fn command() -> Command {
    Command::new("inspect")
        .about("Examine raw stone files")
        .long_about("Show detailed (debug) information on a local `.stone` file")
        .arg(arg!(<PATH> ... "files to inspect").value_parser(clap::value_parser!(PathBuf)))
        .arg(arg!(--check "Check the integrity of the stone file(s)").action(clap::ArgAction::SetTrue))
        .arg(
            arg!(-q --quiet "Suppress output, only exit status indicates success or failure (requires --check)")
                .action(clap::ArgAction::SetTrue)
                .requires("check"),
        )
}

///
/// Inspect the given .stone files and print results
///
pub fn handle(args: &ArgMatches) -> Result<(), Error> {
    let paths = args
        .get_many::<PathBuf>("PATH")
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();

    let check = args.get_flag("check");
    let quiet = args.get_flag("quiet");

    if check {
        handle_check(paths, quiet)
    } else {
        handle_detailed(paths)
    }
}

fn handle_check(paths: Vec<PathBuf>, quiet: bool) -> Result<(), Error> {
    let mut had_error = false;
    for path in paths {
        if !quiet {
            println!("Checking: {:?}", path.display());
        }

        match File::open(&path).map_err(Error::IO).and_then(check_stone_integrity) {
            Ok(payload_kinds) => {
                if !quiet {
                    for kind in payload_kinds {
                        println!("  OK: {kind}");
                    }
                    println!("Result: OK\n");
                }
            }
            Err(e) => {
                had_error = true;
                if !quiet {
                    println!("Result: FAILED - {e}\n");
                }
            }
        }
    }

    if had_error {
        Err(Error::ValidationFailed)
    } else {
        Ok(())
    }
}

fn handle_detailed(paths: Vec<PathBuf>) -> Result<(), Error> {
    // Process each input path in order.
    for path in paths {
        let mut file = File::open(&path)?;
        let mut reader = stone::read(&mut file)?;

        let header = reader.header;
        let payloads = reader.payloads()?;

        // Grab the header version
        print!("{path:?} = stone container version {:?}", header.version());

        for payload in payloads.flatten() {
            let mut layouts = vec![];

            // Grab deps/providers/conflicts
            let mut deps = vec![];
            let mut provs = vec![];
            let mut cnfls = vec![];

            match payload {
                StoneDecodedPayload::Layout(l) => layouts = l.body,
                StoneDecodedPayload::Meta(meta) => {
                    println!();

                    for record in meta.body {
                        let name = format!("{:?}", record.tag);

                        match &record.primitive {
                            StonePayloadMetaPrimitive::Provider(k, p)
                                if record.tag == StonePayloadMetaTag::Provides =>
                            {
                                provs.push(format!("{k}({p})"));
                            }
                            StonePayloadMetaPrimitive::Provider(k, p)
                                if record.tag == StonePayloadMetaTag::Conflicts =>
                            {
                                cnfls.push(format!("{k}({p})"));
                            }
                            StonePayloadMetaPrimitive::Dependency(k, d) => {
                                deps.push(format!("{k}({d})"));
                            }
                            StonePayloadMetaPrimitive::String(s) => {
                                println!("{name:COLUMN_WIDTH$} : {s}");
                            }
                            StonePayloadMetaPrimitive::Int64(i) => {
                                println!("{name:COLUMN_WIDTH$} : {i}");
                            }
                            StonePayloadMetaPrimitive::Uint64(i) => {
                                println!("{name:COLUMN_WIDTH$} : {i}");
                            }
                            _ => {
                                println!("{name:COLUMN_WIDTH$} : {record:?}");
                            }
                        }
                    }
                }
                _ => {}
            }

            if !deps.is_empty() {
                println!("\n{:COLUMN_WIDTH$} :", "Dependencies");
                for dep in deps {
                    println!("    - {dep}");
                }
            }
            if !provs.is_empty() {
                println!("\n{:COLUMN_WIDTH$} :", "Providers");
                for prov in provs {
                    println!("    - {prov}");
                }
            }
            if !cnfls.is_empty() {
                println!("\n{:COLUMN_WIDTH$} :", "Conflicts");
                for cnfl in cnfls {
                    println!("    - {cnfl}");
                }
            }

            if !layouts.is_empty() {
                println!("\n{:COLUMN_WIDTH$} :", "Layout entries");
                for layout in layouts {
                    match layout.file {
                        StonePayloadLayoutFile::Regular(hash, target) => {
                            println!("    - /usr/{target} - [Regular] {hash:032x}");
                        }
                        StonePayloadLayoutFile::Directory(target) => {
                            println!("    - /usr/{target} [Directory]");
                        }
                        StonePayloadLayoutFile::Symlink(source, target) => {
                            println!("    - /usr/{target} -> {source} [Symlink]");
                        }
                        _ => unreachable!(),
                    };
                }
            }
        }
    }
    Ok(())
}

/// Checks the integrity of a single .stone file by reading all payloads
/// and validating their checksums from any readable source.
fn check_stone_integrity(mut source: impl Read + Seek) -> Result<Vec<String>, Error> {
    let mut reader = stone::read(&mut source)?;
    let mut found_payloads = Vec::new();

    // Decode all non-content payloads which validates checksums
    let payloads = reader.payloads()?.collect::<Result<Vec<_>, _>>()?;

    // Find the content payload, if it exists.
    let content_payload = payloads.iter().find_map(StoneDecodedPayload::content);

    // Explicitly unpack the content payload to a null sink to validate its checksum.
    if let Some(content) = content_payload {
        reader.unpack_content(content, &mut sink())?;
    }

    // Collect the names of found payloads for reporting.
    for p in payloads {
        found_payloads.push(p.name().to_owned());
    }

    Ok(found_payloads)
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io")]
    IO(#[from] std::io::Error),

    #[error("stone format")]
    Format(#[from] StoneReadError),

    #[error("One or more files failed the integrity check")]
    ValidationFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const VALID_STONE_BYTES: &[u8] = include_bytes!("../../../test/bash-completion-2.11-1-1-x86_64.stone");

    #[test]
    fn test_check_valid_stone() {
        let source = Cursor::new(VALID_STONE_BYTES);
        let result = check_stone_integrity(source);
        assert!(result.is_ok(), "Check should pass for a valid stone file");

        let payloads = result.unwrap();
        assert!(payloads.contains(&"Meta".to_owned()));
        assert!(payloads.contains(&"Layout".to_owned()));
        assert!(payloads.contains(&"Index".to_owned()));
        assert!(payloads.contains(&"Content".to_owned()));
    }

    #[test]
    fn test_check_corrupted_stone() {
        let mut corrupted_bytes = VALID_STONE_BYTES.to_vec();

        // Corrupt a byte in the middle of the file to trigger corruption detection.
        let mid = corrupted_bytes.len() / 2;
        corrupted_bytes[mid] = corrupted_bytes[mid].wrapping_add(1);

        let source = Cursor::new(corrupted_bytes);
        let result = check_stone_integrity(source);

        assert!(result.is_err(), "Check should fail for a corrupted stone file");

        // Any corruption should be detected - could be checksum mismatch or data corruption
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::Format(StoneReadError::PayloadChecksum { .. }))
                || matches!(err, Error::Format(StoneReadError::Io(_))),
            "Error should be corruption-related, got: {err:?}"
        );
    }

    #[test]
    fn test_check_malformed_stone() {
        // Use garbage data that doesn't even have a valid header.
        let malformed_bytes = b"this is not a stone file";
        let source = Cursor::new(malformed_bytes);
        let result = check_stone_integrity(source);

        assert!(result.is_err(), "Check should fail for malformed data");

        // Check for a header decoding error.
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::Format(StoneReadError::HeaderDecode(_))),
            "Error should be a header decode error"
        );
    }
}
