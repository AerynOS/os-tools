use stone::{StonePayloadMetaDependency, StonePayloadMetaTag};

use crate::StoneString;

#[repr(C)]
pub struct StonePayloadMetaRecord {
    pub tag: StonePayloadMetaTag,
    pub primitive_type: StonePayloadMetaPrimitiveType,
    pub primitive_payload: StonePayloadMetaPrimitivePayload,
}

impl From<&stone::StonePayloadMetaRecord> for StonePayloadMetaRecord {
    fn from(record: &stone::StonePayloadMetaRecord) -> Self {
        Self {
            tag: record.tag,
            primitive_type: match &record.primitive {
                stone::StonePayloadMetaPrimitive::Int8(_) => StonePayloadMetaPrimitiveType::Int8,
                stone::StonePayloadMetaPrimitive::Uint8(_) => StonePayloadMetaPrimitiveType::Uint8,
                stone::StonePayloadMetaPrimitive::Int16(_) => StonePayloadMetaPrimitiveType::Int16,
                stone::StonePayloadMetaPrimitive::Uint16(_) => StonePayloadMetaPrimitiveType::Uint16,
                stone::StonePayloadMetaPrimitive::Int32(_) => StonePayloadMetaPrimitiveType::Int32,
                stone::StonePayloadMetaPrimitive::Uint32(_) => StonePayloadMetaPrimitiveType::Uint32,
                stone::StonePayloadMetaPrimitive::Int64(_) => StonePayloadMetaPrimitiveType::Int64,
                stone::StonePayloadMetaPrimitive::Uint64(_) => StonePayloadMetaPrimitiveType::Uint64,
                stone::StonePayloadMetaPrimitive::String(_) => StonePayloadMetaPrimitiveType::String,
                stone::StonePayloadMetaPrimitive::Dependency(_, _) => StonePayloadMetaPrimitiveType::Dependency,
                stone::StonePayloadMetaPrimitive::Provider(_, _) => StonePayloadMetaPrimitiveType::Provider,
            },
            primitive_payload: match &record.primitive {
                stone::StonePayloadMetaPrimitive::Int8(a) => StonePayloadMetaPrimitivePayload { int8: *a },
                stone::StonePayloadMetaPrimitive::Uint8(a) => StonePayloadMetaPrimitivePayload { uint8: *a },
                stone::StonePayloadMetaPrimitive::Int16(a) => StonePayloadMetaPrimitivePayload { int16: *a },
                stone::StonePayloadMetaPrimitive::Uint16(a) => StonePayloadMetaPrimitivePayload { uint16: *a },
                stone::StonePayloadMetaPrimitive::Int32(a) => StonePayloadMetaPrimitivePayload { int32: *a },
                stone::StonePayloadMetaPrimitive::Uint32(a) => StonePayloadMetaPrimitivePayload { uint32: *a },
                stone::StonePayloadMetaPrimitive::Int64(a) => StonePayloadMetaPrimitivePayload { int64: *a },
                stone::StonePayloadMetaPrimitive::Uint64(a) => StonePayloadMetaPrimitivePayload { uint64: *a },
                stone::StonePayloadMetaPrimitive::String(a) => StonePayloadMetaPrimitivePayload {
                    string: StoneString::new(a),
                },
                stone::StonePayloadMetaPrimitive::Dependency(kind, name) => StonePayloadMetaPrimitivePayload {
                    dependency: StonePayloadMetaDependencyValue {
                        kind: *kind,
                        name: StoneString::new(name),
                    },
                },
                stone::StonePayloadMetaPrimitive::Provider(kind, name) => StonePayloadMetaPrimitivePayload {
                    provider: StonePayloadMetaProviderValue {
                        kind: *kind,
                        name: StoneString::new(name),
                    },
                },
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub enum StonePayloadMetaPrimitiveType {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Int64,
    Uint64,
    String,
    Dependency,
    Provider,
}

#[repr(C)]
pub union StonePayloadMetaPrimitivePayload {
    int8: i8,
    uint8: u8,
    int16: i16,
    uint16: u16,
    int32: i32,
    uint32: u32,
    int64: i64,
    uint64: u64,
    string: StoneString,
    dependency: StonePayloadMetaDependencyValue,
    provider: StonePayloadMetaProviderValue,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StonePayloadMetaDependencyValue {
    pub kind: StonePayloadMetaDependency,
    pub name: StoneString,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StonePayloadMetaProviderValue {
    pub kind: StonePayloadMetaDependency,
    pub name: StoneString,
}
