// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use super::{DecodeError, EncodeError, Record};
use crate::{ReadExt, WriteExt};

/// The Meta payload contains a series of sequential records with
/// strong types and context tags, i.e. their use such as Name.
/// These record all metadata for every .stone packages and provide
/// no content
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub tag: Tag,
    pub kind: Kind,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Dependency {
    /// Just the plain name of a package
    #[strum(serialize = "name")]
    PackageName = 0,

    /// A soname based dependency
    #[strum(serialize = "soname")]
    SharedLibrary,

    /// A pkgconfig `.pc` based dependency
    PkgConfig,

    /// Special interpreter (PT_INTERP/etc) to run the binaries
    Interpreter,

    /// A CMake module
    CMake,

    /// A Python module
    Python,

    /// A binary in /usr/bin
    Binary,

    /// A binary in /usr/sbin
    #[strum(serialize = "sysbinary")]
    SystemBinary,

    /// An emul32-compatible pkgconfig .pc dependency (lib32/*.pc)
    PkgConfig32,
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Int8(i8),
    Uint8(u8),
    Int16(i16),
    Uint16(u16),
    Int32(i32),
    Uint32(u32),
    Int64(i64),
    Uint64(u64),
    String(String),
    Dependency(Dependency, String),
    Provider(Dependency, String),
}

impl Kind {
    fn size(&self) -> usize {
        match self {
            Kind::Int8(_) => size_of::<i8>(),
            Kind::Uint8(_) => size_of::<u8>(),
            Kind::Int16(_) => size_of::<i16>(),
            Kind::Uint16(_) => size_of::<u16>(),
            Kind::Int32(_) => size_of::<i32>(),
            Kind::Uint32(_) => size_of::<u32>(),
            Kind::Int64(_) => size_of::<i64>(),
            Kind::Uint64(_) => size_of::<u64>(),
            // nul terminator
            Kind::String(s) => s.len() + 1,
            // Plus dep size & nul terminator
            Kind::Dependency(_, s) => s.len() + 2,
            // Plus dep size & nul terminator
            Kind::Provider(_, s) => s.len() + 2,
        }
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    // Name of the package
    Name = 1,
    // Architecture of the package
    Architecture = 2,
    // Version of the package
    Version = 3,
    // Summary of the package
    Summary = 4,
    // Description of the package
    Description = 5,
    // Homepage for the package
    Homepage = 6,
    // ID for the source package, used for grouping
    SourceID = 7,
    // Runtime dependencies
    Depends = 8,
    // Provides some capability or name
    Provides = 9,
    // Conflicts with some capability or name
    Conflicts = 10,
    // Release number for the package
    Release = 11,
    // SPDX license identifier
    License = 12,
    // Currently recorded build number
    BuildRelease = 13,
    // Repository index specific (relative URI)
    PackageURI = 14,
    // Repository index specific (Package hash)
    PackageHash = 15,
    // Repository index specific (size on disk)
    PackageSize = 16,
    // A Build Dependency
    BuildDepends = 17,
    // Upstream URI for the source
    SourceURI = 18,
    // Relative path for the source within the upstream URI
    SourcePath = 19,
    // Ref/commit of the upstream source
    SourceRef = 20,
}

/// Helper to decode a dependency's encoded kind
fn decode_dependency(i: u8) -> Result<Dependency, DecodeError> {
    let result = match i {
        0 => Dependency::PackageName,
        1 => Dependency::SharedLibrary,
        2 => Dependency::PkgConfig,
        3 => Dependency::Interpreter,
        4 => Dependency::CMake,
        5 => Dependency::Python,
        6 => Dependency::Binary,
        7 => Dependency::SystemBinary,
        8 => Dependency::PkgConfig32,
        _ => return Err(DecodeError::UnknownDependency(i)),
    };
    Ok(result)
}

impl Record for Meta {
    fn decode<R: Read>(mut reader: R) -> Result<Self, DecodeError> {
        let length = reader.read_u32()?;

        let tag = match reader.read_u16()? {
            1 => Tag::Name,
            2 => Tag::Architecture,
            3 => Tag::Version,
            4 => Tag::Summary,
            5 => Tag::Description,
            6 => Tag::Homepage,
            7 => Tag::SourceID,
            8 => Tag::Depends,
            9 => Tag::Provides,
            10 => Tag::Conflicts,
            11 => Tag::Release,
            12 => Tag::License,
            13 => Tag::BuildRelease,
            14 => Tag::PackageURI,
            15 => Tag::PackageHash,
            16 => Tag::PackageSize,
            17 => Tag::BuildDepends,
            18 => Tag::SourceURI,
            19 => Tag::SourcePath,
            20 => Tag::SourceRef,
            t => return Err(DecodeError::UnknownMetaTag(t)),
        };

        let kind = reader.read_u8()?;
        let _padding = reader.read_array::<1>()?;

        // Remove null terminated byte from string
        let sanitize = |s: String| s.trim_end_matches('\0').to_owned();

        let kind = match kind {
            1 => Kind::Int8(reader.read_u8()? as i8),
            2 => Kind::Uint8(reader.read_u8()?),
            3 => Kind::Int16(reader.read_u16()? as i16),
            4 => Kind::Uint16(reader.read_u16()?),
            5 => Kind::Int32(reader.read_u32()? as i32),
            6 => Kind::Uint32(reader.read_u32()?),
            7 => Kind::Int64(reader.read_u64()? as i64),
            8 => Kind::Uint64(reader.read_u64()?),
            9 => Kind::String(sanitize(reader.read_string(length as u64)?)),
            10 => Kind::Dependency(
                // DependencyKind u8 subtracted from length
                decode_dependency(reader.read_u8()?)?,
                sanitize(reader.read_string(length as u64 - 1)?),
            ),
            11 => Kind::Provider(
                // DependencyKind u8 subtracted from length
                decode_dependency(reader.read_u8()?)?,
                sanitize(reader.read_string(length as u64 - 1)?),
            ),
            k => return Err(DecodeError::UnknownMetaKind(k)),
        };

        Ok(Self { tag, kind })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), EncodeError> {
        let kind = match self.kind {
            Kind::Int8(_) => 1,
            Kind::Uint8(_) => 2,
            Kind::Int16(_) => 3,
            Kind::Uint16(_) => 4,
            Kind::Int32(_) => 5,
            Kind::Uint32(_) => 6,
            Kind::Int64(_) => 7,
            Kind::Uint64(_) => 8,
            Kind::String(_) => 9,
            Kind::Dependency(_, _) => 10,
            Kind::Provider(_, _) => 11,
        };

        writer.write_u32(self.kind.size() as u32)?;
        writer.write_u16(self.tag as u16)?;
        writer.write_u8(kind)?;
        // Padding
        writer.write_array::<1>([0])?;

        match &self.kind {
            Kind::Int8(i) => writer.write_u8(*i as u8)?,
            Kind::Uint8(i) => writer.write_u8(*i)?,
            Kind::Int16(i) => writer.write_u16(*i as u16)?,
            Kind::Uint16(i) => writer.write_u16(*i)?,
            Kind::Int32(i) => writer.write_u32(*i as u32)?,
            Kind::Uint32(i) => writer.write_u32(*i)?,
            Kind::Int64(i) => writer.write_u64(*i as u64)?,
            Kind::Uint64(i) => writer.write_u64(*i)?,
            Kind::String(s) => {
                writer.write_all(s.as_bytes())?;
                writer.write_u8(b'\0')?;
            }
            Kind::Dependency(dep, s) | Kind::Provider(dep, s) => {
                writer.write_u8(*dep as u8)?;
                writer.write_all(s.as_bytes())?;
                writer.write_u8(b'\0')?;
            }
        }

        Ok(())
    }

    fn size(&self) -> usize {
        4 + 2 + 1 + 1 + self.kind.size()
    }
}
