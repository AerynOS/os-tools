// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};

use super::{Record, StonePayloadDecodeError, StonePayloadEncodeError};
use crate::ext::{ReadExt, WriteExt};

/// The Meta payload contains a series of sequential records with
/// strong types and context tags, i.e. their use such as Name.
/// These record all metadata for every .stone packages and provide
/// no content
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StonePayloadMetaRecord {
    pub tag: StonePayloadMetaTag,
    pub primitive: StonePayloadMetaPrimitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
#[repr(u8)]
pub enum StonePayloadMetaDependency {
    /// Just the plain name of a package
    #[strum(serialize = "name")]
    PackageName = 0,

    /// A soname based dependency
    #[strum(serialize = "soname")]
    SharedLibrary = 1,

    /// A pkgconfig `.pc` based dependency
    PkgConfig = 2,

    /// Special interpreter (PT_INTERP/etc) to run the binaries
    Interpreter = 3,

    /// A CMake module
    CMake = 4,

    /// A Python module
    Python = 5,

    /// A binary in /usr/bin
    Binary = 6,

    /// A binary in /usr/sbin
    #[strum(serialize = "sysbinary")]
    SystemBinary = 7,

    /// An emul32-compatible pkgconfig .pc dependency (lib32*.pc)
    PkgConfig32 = 8,

    Unknown = 255,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StonePayloadMetaPrimitive {
    Int8(i8),
    Uint8(u8),
    Int16(i16),
    Uint16(u16),
    Int32(i32),
    Uint32(u32),
    Int64(i64),
    Uint64(u64),
    String(String),
    Dependency(StonePayloadMetaDependency, String),
    Provider(StonePayloadMetaDependency, String),
    Unknown(Vec<u8>),
}

impl StonePayloadMetaPrimitive {
    fn size(&self) -> usize {
        match self {
            StonePayloadMetaPrimitive::Int8(_) => size_of::<i8>(),
            StonePayloadMetaPrimitive::Uint8(_) => size_of::<u8>(),
            StonePayloadMetaPrimitive::Int16(_) => size_of::<i16>(),
            StonePayloadMetaPrimitive::Uint16(_) => size_of::<u16>(),
            StonePayloadMetaPrimitive::Int32(_) => size_of::<i32>(),
            StonePayloadMetaPrimitive::Uint32(_) => size_of::<u32>(),
            StonePayloadMetaPrimitive::Int64(_) => size_of::<i64>(),
            StonePayloadMetaPrimitive::Uint64(_) => size_of::<u64>(),
            // nul terminator
            StonePayloadMetaPrimitive::String(s) => s.len() + 1,
            // Plus dep size & nul terminator
            StonePayloadMetaPrimitive::Dependency(_, s) => s.len() + 2,
            // Plus dep size & nul terminator
            StonePayloadMetaPrimitive::Provider(_, s) => s.len() + 2,
            StonePayloadMetaPrimitive::Unknown(data) => data.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
#[repr(u16)]
pub enum StonePayloadMetaTag {
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

    Unknown = u16::MAX,
}

/// Helper to decode a dependency's encoded kind
fn decode_dependency(i: u8) -> StonePayloadMetaDependency {
    match i {
        0 => StonePayloadMetaDependency::PackageName,
        1 => StonePayloadMetaDependency::SharedLibrary,
        2 => StonePayloadMetaDependency::PkgConfig,
        3 => StonePayloadMetaDependency::Interpreter,
        4 => StonePayloadMetaDependency::CMake,
        5 => StonePayloadMetaDependency::Python,
        6 => StonePayloadMetaDependency::Binary,
        7 => StonePayloadMetaDependency::SystemBinary,
        8 => StonePayloadMetaDependency::PkgConfig32,
        _ => StonePayloadMetaDependency::Unknown,
    }
}

impl Record for StonePayloadMetaRecord {
    fn decode<R: Read>(mut reader: R) -> Result<Self, StonePayloadDecodeError> {
        let length = reader.read_u32()?;

        let tag = match reader.read_u16()? {
            1 => StonePayloadMetaTag::Name,
            2 => StonePayloadMetaTag::Architecture,
            3 => StonePayloadMetaTag::Version,
            4 => StonePayloadMetaTag::Summary,
            5 => StonePayloadMetaTag::Description,
            6 => StonePayloadMetaTag::Homepage,
            7 => StonePayloadMetaTag::SourceID,
            8 => StonePayloadMetaTag::Depends,
            9 => StonePayloadMetaTag::Provides,
            10 => StonePayloadMetaTag::Conflicts,
            11 => StonePayloadMetaTag::Release,
            12 => StonePayloadMetaTag::License,
            13 => StonePayloadMetaTag::BuildRelease,
            14 => StonePayloadMetaTag::PackageURI,
            15 => StonePayloadMetaTag::PackageHash,
            16 => StonePayloadMetaTag::PackageSize,
            17 => StonePayloadMetaTag::BuildDepends,
            18 => StonePayloadMetaTag::SourceURI,
            19 => StonePayloadMetaTag::SourcePath,
            20 => StonePayloadMetaTag::SourceRef,
            _ => StonePayloadMetaTag::Unknown,
        };

        let kind = reader.read_u8()?;
        let _padding = reader.read_array_::<1>()?;

        // Remove null terminated byte from string
        let sanitize = |s: String| s.trim_end_matches('\0').to_owned();

        let primitive = match kind {
            1 => StonePayloadMetaPrimitive::Int8(reader.read_u8()? as i8),
            2 => StonePayloadMetaPrimitive::Uint8(reader.read_u8()?),
            3 => StonePayloadMetaPrimitive::Int16(reader.read_u16()? as i16),
            4 => StonePayloadMetaPrimitive::Uint16(reader.read_u16()?),
            5 => StonePayloadMetaPrimitive::Int32(reader.read_u32()? as i32),
            6 => StonePayloadMetaPrimitive::Uint32(reader.read_u32()?),
            7 => StonePayloadMetaPrimitive::Int64(reader.read_u64()? as i64),
            8 => StonePayloadMetaPrimitive::Uint64(reader.read_u64()?),
            9 => StonePayloadMetaPrimitive::String(sanitize(reader.read_string(length as u64)?)),
            10 => StonePayloadMetaPrimitive::Dependency(
                // DependencyKind u8 subtracted from length
                decode_dependency(reader.read_u8()?),
                sanitize(reader.read_string(length as u64 - 1)?),
            ),
            11 => StonePayloadMetaPrimitive::Provider(
                // DependencyKind u8 subtracted from length
                decode_dependency(reader.read_u8()?),
                sanitize(reader.read_string(length as u64 - 1)?),
            ),
            _ => StonePayloadMetaPrimitive::Unknown(reader.read_vec(length as usize)?),
        };

        Ok(Self { tag, primitive })
    }

    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), StonePayloadEncodeError> {
        let kind = match self.primitive {
            StonePayloadMetaPrimitive::Int8(_) => 1,
            StonePayloadMetaPrimitive::Uint8(_) => 2,
            StonePayloadMetaPrimitive::Int16(_) => 3,
            StonePayloadMetaPrimitive::Uint16(_) => 4,
            StonePayloadMetaPrimitive::Int32(_) => 5,
            StonePayloadMetaPrimitive::Uint32(_) => 6,
            StonePayloadMetaPrimitive::Int64(_) => 7,
            StonePayloadMetaPrimitive::Uint64(_) => 8,
            StonePayloadMetaPrimitive::String(_) => 9,
            StonePayloadMetaPrimitive::Dependency(_, _) => 10,
            StonePayloadMetaPrimitive::Provider(_, _) => 11,
            StonePayloadMetaPrimitive::Unknown(..) => 255,
        };

        writer.write_u32(self.primitive.size() as u32)?;
        writer.write_u16(self.tag as u16)?;
        writer.write_u8(kind)?;
        // Padding
        writer.write_array::<1>([0])?;

        match &self.primitive {
            StonePayloadMetaPrimitive::Int8(i) => writer.write_u8(*i as u8)?,
            StonePayloadMetaPrimitive::Uint8(i) => writer.write_u8(*i)?,
            StonePayloadMetaPrimitive::Int16(i) => writer.write_u16(*i as u16)?,
            StonePayloadMetaPrimitive::Uint16(i) => writer.write_u16(*i)?,
            StonePayloadMetaPrimitive::Int32(i) => writer.write_u32(*i as u32)?,
            StonePayloadMetaPrimitive::Uint32(i) => writer.write_u32(*i)?,
            StonePayloadMetaPrimitive::Int64(i) => writer.write_u64(*i as u64)?,
            StonePayloadMetaPrimitive::Uint64(i) => writer.write_u64(*i)?,
            StonePayloadMetaPrimitive::String(s) => {
                writer.write_all(s.as_bytes())?;
                writer.write_u8(b'\0')?;
            }
            StonePayloadMetaPrimitive::Dependency(dep, s) | StonePayloadMetaPrimitive::Provider(dep, s) => {
                writer.write_u8(*dep as u8)?;
                writer.write_all(s.as_bytes())?;
                writer.write_u8(b'\0')?;
            }
            StonePayloadMetaPrimitive::Unknown(data) => {
                writer.write_all(data)?;
            }
        }

        Ok(())
    }

    fn size(&self) -> usize {
        4 + 2 + 1 + 1 + self.primitive.size()
    }
}
