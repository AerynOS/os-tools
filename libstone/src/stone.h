// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0


#ifndef STONE_H
#define STONE_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define STONE_HEADER_SIZE 32

enum StoneSeekFrom
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  STONE_SEEK_FROM_START = 0,
  STONE_SEEK_FROM_CURRENT = 1,
  STONE_SEEK_FROM_END = 2,
};
#ifndef __cplusplus
typedef uint8_t StoneSeekFrom;
#endif // __cplusplus

/**
 * Format versions are defined as u32, to allow further mangling in future
 */
enum StoneHeaderVersion
#ifdef __cplusplus
  : uint32_t
#endif // __cplusplus
 {
  STONE_HEADER_VERSION_V1 = 1,
};
#ifndef __cplusplus
typedef uint32_t StoneHeaderVersion;
#endif // __cplusplus

/**
 * Well known file type for a v1 stone container
 *
 * Some types are now legacy as we're going to use Ion to define them.
 *
 */
enum StoneHeaderV1FileType
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  /**
   * Binary package
   */
  STONE_HEADER_V1_FILE_TYPE_BINARY = 1,
  /**
   * Delta package
   */
  STONE_HEADER_V1_FILE_TYPE_DELTA = 2,
  /**
   * (Legacy) repository index
   */
  STONE_HEADER_V1_FILE_TYPE_REPOSITORY = 3,
  /**
   * (Legacy) build manifest
   */
  STONE_HEADER_V1_FILE_TYPE_BUILD_MANIFEST = 4,
  STONE_HEADER_V1_FILE_TYPE_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StoneHeaderV1FileType;
#endif // __cplusplus

enum StonePayloadKind
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  STONE_PAYLOAD_KIND_META = 1,
  STONE_PAYLOAD_KIND_CONTENT = 2,
  STONE_PAYLOAD_KIND_LAYOUT = 3,
  STONE_PAYLOAD_KIND_INDEX = 4,
  STONE_PAYLOAD_KIND_ATTRIBUTES = 5,
  STONE_PAYLOAD_KIND_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StonePayloadKind;
#endif // __cplusplus

enum StonePayloadCompression
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  STONE_PAYLOAD_COMPRESSION_NONE = 1,
  STONE_PAYLOAD_COMPRESSION_ZSTD = 2,
  STONE_PAYLOAD_COMPRESSION_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StonePayloadCompression;
#endif // __cplusplus

/**
 * Layout entries record their target file type so they can be rebuilt on
 * the target installation.
 */
enum StonePayloadLayoutFileType
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  /**
   * Regular file
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_REGULAR = 1,
  /**
   * Symbolic link (source + target set)
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_SYMLINK = 2,
  /**
   * Directory node
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_DIRECTORY = 3,
  /**
   * Character device
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_CHARACTER_DEVICE = 4,
  /**
   * Block device
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_BLOCK_DEVICE = 5,
  /**
   * FIFO node
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_FIFO = 6,
  /**
   * UNIX Socket
   */
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_SOCKET = 7,
  STONE_PAYLOAD_LAYOUT_FILE_TYPE_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StonePayloadLayoutFileType;
#endif // __cplusplus

enum StonePayloadMetaTag
#ifdef __cplusplus
  : uint16_t
#endif // __cplusplus
 {
  STONE_PAYLOAD_META_TAG_NAME = 1,
  STONE_PAYLOAD_META_TAG_ARCHITECTURE = 2,
  STONE_PAYLOAD_META_TAG_VERSION = 3,
  STONE_PAYLOAD_META_TAG_SUMMARY = 4,
  STONE_PAYLOAD_META_TAG_DESCRIPTION = 5,
  STONE_PAYLOAD_META_TAG_HOMEPAGE = 6,
  STONE_PAYLOAD_META_TAG_SOURCE_ID = 7,
  STONE_PAYLOAD_META_TAG_DEPENDS = 8,
  STONE_PAYLOAD_META_TAG_PROVIDES = 9,
  STONE_PAYLOAD_META_TAG_CONFLICTS = 10,
  STONE_PAYLOAD_META_TAG_RELEASE = 11,
  STONE_PAYLOAD_META_TAG_LICENSE = 12,
  STONE_PAYLOAD_META_TAG_BUILD_RELEASE = 13,
  STONE_PAYLOAD_META_TAG_PACKAGE_URI = 14,
  STONE_PAYLOAD_META_TAG_PACKAGE_HASH = 15,
  STONE_PAYLOAD_META_TAG_PACKAGE_SIZE = 16,
  STONE_PAYLOAD_META_TAG_BUILD_DEPENDS = 17,
  STONE_PAYLOAD_META_TAG_SOURCE_URI = 18,
  STONE_PAYLOAD_META_TAG_SOURCE_PATH = 19,
  STONE_PAYLOAD_META_TAG_SOURCE_REF = 20,
  STONE_PAYLOAD_META_TAG_UNKNOWN = UINT16_MAX,
};
#ifndef __cplusplus
typedef uint16_t StonePayloadMetaTag;
#endif // __cplusplus

enum StonePayloadMetaPrimitiveType
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT8 = 1,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT8 = 2,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT16 = 3,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT16 = 4,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT32 = 5,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT32 = 6,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT64 = 7,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT64 = 8,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_STRING = 9,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_DEPENDENCY = 10,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_PROVIDER = 11,
  STONE_PAYLOAD_META_PRIMITIVE_TYPE_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StonePayloadMetaPrimitiveType;
#endif // __cplusplus

enum StonePayloadMetaDependency
#ifdef __cplusplus
  : uint8_t
#endif // __cplusplus
 {
  /**
   * Just the plain name of a package
   */
  STONE_PAYLOAD_META_DEPENDENCY_PACKAGE_NAME = 0,
  /**
   * A soname based dependency
   */
  STONE_PAYLOAD_META_DEPENDENCY_SHARED_LIBRARY = 1,
  /**
   * A pkgconfig `.pc` based dependency
   */
  STONE_PAYLOAD_META_DEPENDENCY_PKG_CONFIG = 2,
  /**
   * Special interpreter (PT_INTERP/etc) to run the binaries
   */
  STONE_PAYLOAD_META_DEPENDENCY_INTERPRETER = 3,
  /**
   * A CMake module
   */
  STONE_PAYLOAD_META_DEPENDENCY_C_MAKE = 4,
  /**
   * A Python module
   */
  STONE_PAYLOAD_META_DEPENDENCY_PYTHON = 5,
  /**
   * A binary in /usr/bin
   */
  STONE_PAYLOAD_META_DEPENDENCY_BINARY = 6,
  /**
   * A binary in /usr/sbin
   */
  STONE_PAYLOAD_META_DEPENDENCY_SYSTEM_BINARY = 7,
  /**
   * An emul32-compatible pkgconfig .pc dependency (lib32*.pc)
   */
  STONE_PAYLOAD_META_DEPENDENCY_PKG_CONFIG32 = 8,
  STONE_PAYLOAD_META_DEPENDENCY_UNKNOWN = 255,
};
#ifndef __cplusplus
typedef uint8_t StonePayloadMetaDependency;
#endif // __cplusplus

typedef struct StonePayload StonePayload;

typedef struct StonePayloadContentReader StonePayloadContentReader;

typedef struct StoneReader StoneReader;

typedef struct StoneReadVTable {
  uintptr_t (*read)(void*, char*, uintptr_t);
  int64_t (*seek)(void*, int64_t, StoneSeekFrom);
} StoneReadVTable;

typedef struct StoneReader StoneReader;

/**
 * Header for the v1 format version
 */
typedef struct StoneHeaderV1 {
  uint16_t num_payloads;
  StoneHeaderV1FileType file_type;
} StoneHeaderV1;

typedef struct StonePayloadContentReader StonePayloadContentReader;

typedef struct StonePayloadHeader {
  uint64_t stored_size;
  uint64_t plain_size;
  uint8_t checksum[8];
  uintptr_t num_records;
  uint16_t version;
  StonePayloadKind kind;
  StonePayloadCompression compression;
} StonePayloadHeader;

typedef struct StoneString {
  const uint8_t *buf;
  size_t size;
} StoneString;

typedef struct StonePayloadLayoutFileRegular {
  uint8_t hash[16];
  struct StoneString name;
} StonePayloadLayoutFileRegular;

typedef struct StonePayloadLayoutFileSymlink {
  struct StoneString source;
  struct StoneString target;
} StonePayloadLayoutFileSymlink;

typedef union StonePayloadLayoutFilePayload {
  struct StonePayloadLayoutFileRegular regular;
  struct StonePayloadLayoutFileSymlink symlink;
  struct StoneString directory;
  struct StoneString character_device;
  struct StoneString block_device;
  struct StoneString fifo;
  struct StoneString socket;
} StonePayloadLayoutFilePayload;

typedef struct StonePayloadLayoutRecord {
  uint32_t uid;
  uint32_t gid;
  uint32_t mode;
  uint32_t tag;
  StonePayloadLayoutFileType file_type;
  union StonePayloadLayoutFilePayload file_payload;
} StonePayloadLayoutRecord;

typedef struct StonePayloadMetaDependencyValue {
  StonePayloadMetaDependency kind;
  struct StoneString name;
} StonePayloadMetaDependencyValue;

typedef struct StonePayloadMetaProviderValue {
  StonePayloadMetaDependency kind;
  struct StoneString name;
} StonePayloadMetaProviderValue;

typedef union StonePayloadMetaPrimitivePayload {
  int8_t int8;
  uint8_t uint8;
  int16_t int16;
  uint16_t uint16;
  int32_t int32;
  uint32_t uint32;
  int64_t int64;
  uint64_t uint64;
  struct StoneString string;
  struct StonePayloadMetaDependencyValue dependency;
  struct StonePayloadMetaProviderValue provider;
} StonePayloadMetaPrimitivePayload;

typedef struct StonePayloadMetaRecord {
  StonePayloadMetaTag tag;
  StonePayloadMetaPrimitiveType primitive_type;
  union StonePayloadMetaPrimitivePayload primitive_payload;
} StonePayloadMetaRecord;

typedef struct StonePayloadIndexRecord {
  uint64_t start;
  uint64_t end;
  uint8_t digest[16];
} StonePayloadIndexRecord;

typedef struct StonePayloadAttributeRecord {
  uintptr_t key_size;
  const uint8_t *key_buf;
  uintptr_t value_size;
  const uint8_t *value_buf;
} StonePayloadAttributeRecord;



#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

int stone_read(void *data,
               struct StoneReadVTable vtable,
               StoneReader **reader_ptr,
               StoneHeaderVersion *version);

int stone_read_file(int file, StoneReader **reader_ptr, StoneHeaderVersion *version);

int stone_read_buf(const uint8_t *buf,
                   uintptr_t len,
                   StoneReader **reader_ptr,
                   StoneHeaderVersion *version);

int stone_reader_header_v1(const StoneReader *reader, struct StoneHeaderV1 *header);

int stone_reader_next_payload(StoneReader *reader, struct StonePayload **payload_ptr);

int stone_reader_unpack_content_payload(StoneReader *reader,
                                        const struct StonePayload *payload,
                                        int file);

int stone_reader_read_content_payload(StoneReader *reader,
                                      const struct StonePayload *payload,
                                      StonePayloadContentReader **content_reader);

void stone_reader_destroy(StoneReader *reader);

size_t stone_payload_content_reader_read(StonePayloadContentReader *content_reader,
                                         uint8_t *buf,
                                         size_t size);

int stone_payload_content_reader_buf_hint(const StonePayloadContentReader *content_reader,
                                          uintptr_t *hint);

int stone_payload_content_reader_is_checksum_valid(const StonePayloadContentReader *content_reader);

void stone_payload_content_reader_destroy(StonePayloadContentReader *content_reader);

int stone_payload_header(const struct StonePayload *payload, struct StonePayloadHeader *header);

int stone_payload_next_layout_record(struct StonePayload *payload,
                                     struct StonePayloadLayoutRecord *record);

int stone_payload_next_meta_record(struct StonePayload *payload,
                                   struct StonePayloadMetaRecord *record);

int stone_payload_next_index_record(struct StonePayload *payload,
                                    struct StonePayloadIndexRecord *record);

int stone_payload_next_attribute_record(struct StonePayload *payload,
                                        struct StonePayloadAttributeRecord *record);

void stone_payload_destroy(struct StonePayload *payload);

void stone_format_header_v1_file_type(StoneHeaderV1FileType file_type, uint8_t *buf);

void stone_format_payload_compression(StonePayloadCompression compression, uint8_t *buf);

void stone_format_payload_kind(StonePayloadKind kind, uint8_t *buf);

void stone_format_payload_layout_file_type(StonePayloadLayoutFileType file_type, uint8_t *buf);

void stone_format_payload_meta_tag(StonePayloadMetaTag tag, uint8_t *buf);

void stone_format_payload_meta_dependency(StonePayloadMetaDependency dependency, uint8_t *buf);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* STONE_H */
