#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <stone.h>
#include <string.h>
#include <unistd.h>

void print_header_v1(StoneHeaderV1 *header) {
  uint8_t file_type[100];

  stone_format_header_v1_file_type(header->file_type, file_type);

  printf("StoneHeaderV1 {\n");
  printf("  num_payloads: %d\n", header->num_payloads);
  printf("  file_type: %s\n", file_type);
  printf("}\n");
}

void print_payload_header(StonePayloadHeader *header) {
  uint8_t compression[100];
  uint8_t kind[100];

  stone_format_payload_compression(header->compression, compression);
  stone_format_payload_kind(header->kind, &(kind[0]));

  printf("StonePayload {\n");
  printf("  kind: %s\n", kind);
  printf("  plain_size: %ld\n", header->plain_size);
  printf("  stored_size: %ld\n", header->stored_size);
  printf("  compression: %s\n", compression);
  printf("  num_records: %ld\n", header->num_records);
  printf("  version: %d\n", header->version);
  printf("}\n");
}

void format_digest(uint8_t digest[16], char (*formatted)[32]) {
  for (size_t i = 0; i < 16; i++) {
    sprintf((char *)formatted + i * 2, "%.02x", digest[i]);
  }
}

void print_payload_layout_record(StonePayloadLayoutRecord *record) {
  uint8_t file_type[100];
  char digest[32];

  stone_format_payload_layout_file_type(record->file_type, file_type);

  printf("StonePayloadLayoutRecord {\n");
  printf("  uid: %d\n", record->uid);
  printf("  gid: %d\n", record->gid);
  printf("  mode: %d\n", record->mode);
  printf("  tag: %d\n", record->tag);
  printf("  file_type: %s\n", file_type);

  switch (record->file_type) {
  case STONE_PAYLOAD_LAYOUT_FILE_TYPE_REGULAR: {
    format_digest(record->file_payload.regular.hash, &digest);
    printf("  hash: %.32s\n", digest);
    printf("  name: %.*s\n", (int)record->file_payload.regular.name.size,
           record->file_payload.regular.name.buf);
    break;
  }
  case STONE_PAYLOAD_LAYOUT_FILE_TYPE_SYMLINK: {
    printf("  source: %.*s\n", (int)record->file_payload.symlink.source.size,
           record->file_payload.symlink.source.buf);
    printf("  target: %.*s\n", (int)record->file_payload.symlink.target.size,
           record->file_payload.symlink.target.buf);
    break;
  }
  }

  printf("}\n");
}

void print_payload_meta_record(StonePayloadMetaRecord *record) {
  uint8_t tag[100];

  stone_format_payload_meta_tag(record->tag, tag);

  printf("StonePayloadMetaRecord {\n");
  printf("  tag: %s\n", tag);

  switch (record->primitive_type) {
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT8: {
    printf("  int8: %d\n", record->primitive_payload.int8);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT8: {
    printf("  uint8: %d\n", record->primitive_payload.uint8);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT16: {
    printf("  int16: %d\n", record->primitive_payload.int16);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT16: {
    printf("  uint16: %d\n", record->primitive_payload.uint16);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT32: {
    printf("  int32: %d\n", record->primitive_payload.int32);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT32: {
    printf("  uint32: %d\n", record->primitive_payload.uint32);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_INT64: {
    printf("  int64: %ld\n", record->primitive_payload.int64);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_UINT64: {
    printf("  uint64: %ld\n", record->primitive_payload.uint64);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_STRING: {
    printf("  string: %.*s\n", (int)record->primitive_payload.string.size,
           record->primitive_payload.string.buf);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_DEPENDENCY: {
    uint8_t dependency[100];

    stone_format_payload_meta_dependency(
        record->primitive_payload.dependency.kind, dependency);

    printf("  dependency: %s(%.*s)\n", dependency,
           (int)record->primitive_payload.dependency.name.size,
           record->primitive_payload.dependency.name.buf);
    break;
  }
  case STONE_PAYLOAD_META_PRIMITIVE_TYPE_PROVIDER: {
    uint8_t provider[100];

    stone_format_payload_meta_dependency(
        record->primitive_payload.provider.kind, provider);

    printf("  provider: %s(%.*s)\n", provider,
           (int)record->primitive_payload.provider.name.size,
           record->primitive_payload.provider.name.buf);
    break;
  }
  }

  printf("}\n");
}

void print_payload_index_record(StonePayloadIndexRecord *record) {
  char digest[32];

  format_digest(record->digest, &digest);

  printf("StonePayloadIndexRecord {\n");
  printf("  start: %ld\n", record->start);
  printf("  end: %ld\n", record->end);
  printf("  digest: %.32s\n", digest);
  printf("}\n");
}

void print_payload_attribute_record(StonePayloadAttributeRecord *record) {
  printf("StonePayloadAttributeRecord {\n");
  printf("  key_size: %ld\n", record->key_size);
  printf("  value_size: %ld\n", record->value_size);
  printf("}\n");
}

void print_inspect_output(StonePayloadMetaRecord *metas, int num_metas,
                          StonePayloadLayoutRecord *layouts, int num_layotus) {

  for (int i = 0; i < num_metas; i++) {
    StonePayloadMetaRecord *meta = &metas[i];
  }
}

void process_records(StonePayload *payload, StonePayloadHeader *payload_header,
                     void **records, int *num_records, int record_size,
                     int (*next_record)(StonePayload *, void *),
                     void (*print_record)(void *)) {

  void *record;
  int i = 0;

  *records =
      realloc(*records, sizeof(StonePayloadLayoutRecord) *
                            (*num_records + payload_header->num_records));
  if (records == NULL) {
    exit(1);
  }

  while (next_record(payload, record = *records +
                                       (*num_records + i) * record_size) >= 0) {
    print_record(record);
    i++;
  }

  *num_records += payload_header->num_records;
}

void process_reader(StoneReader *reader, StoneHeaderVersion version) {
  StoneHeaderV1 header = {0};
  StonePayload **payloads = NULL;
  StonePayloadLayoutRecord *layouts = NULL;
  StonePayloadMetaRecord *metas = NULL;
  StonePayloadIndexRecord *indexes = NULL;
  StonePayloadAttributeRecord *attributes = NULL;
  int num_layouts = 0, num_metas = 0, num_indexes = 0, num_attributes = 0,
      current_payload = 0;

  assert(version == STONE_HEADER_VERSION_V1);

  stone_reader_header_v1(reader, &header);
  print_header_v1(&header);

  payloads = calloc(header.num_payloads, sizeof(StonePayload *));
  if (payloads == NULL) {
    exit(1);
  }

  while (stone_reader_next_payload(reader, &payloads[current_payload]) >= 0) {
    StonePayload *payload = payloads[current_payload];
    StonePayloadHeader payload_header = {0};

    stone_payload_header(payload, &payload_header);
    print_payload_header(&payload_header);

    if (payload_header.compression == STONE_PAYLOAD_COMPRESSION_UNKNOWN) {
      current_payload += 1;
      continue;
    }

    switch (payload_header.kind) {
    case STONE_PAYLOAD_KIND_LAYOUT: {
      process_records(payload, &payload_header, (void *)&layouts, &num_layouts,
                      sizeof(StonePayloadLayoutRecord),
                      (void *)stone_payload_next_layout_record,
                      (void *)print_payload_layout_record);
      break;
    }
    case STONE_PAYLOAD_KIND_META: {
      process_records(payload, &payload_header, (void *)&metas, &num_metas,
                      sizeof(StonePayloadMetaRecord),
                      (void *)stone_payload_next_meta_record,
                      (void *)print_payload_meta_record);
      break;
    }
    case STONE_PAYLOAD_KIND_INDEX: {
      process_records(payload, &payload_header, (void *)&indexes, &num_indexes,
                      sizeof(StonePayloadIndexRecord),
                      (void *)stone_payload_next_index_record,
                      (void *)print_payload_index_record);
      break;
    }
    case STONE_PAYLOAD_KIND_ATTRIBUTES: {
      process_records(payload, &payload_header, (void *)&attributes,
                      &num_attributes, sizeof(StonePayloadAttributeRecord),
                      (void *)stone_payload_next_attribute_record,
                      (void *)print_payload_attribute_record);
      break;
    }
    case STONE_PAYLOAD_KIND_CONTENT: {
      FILE *fptr;
      StonePayloadContentReader *content_reader;
      fptr = fopen("/dev/null", "w+");
      char *buf;
      uint64_t buf_hint = 0;
      int read = 0;

      // We can instead unpack directly to file as a convenience
      // stone_reader_unpack_content_payload(reader, payload, fileno(fptr));

      stone_reader_read_content_payload(reader, payload, &content_reader);

      stone_payload_content_reader_buf_hint(content_reader, &buf_hint);

      buf_hint = buf_hint > 0 ? buf_hint : 1024;
      printf("Unpacking w/ buffer size: %ld\n", buf_hint);

      buf = malloc(buf_hint);
      if (buf == NULL) {
        exit(1);
      }

      while ((read = stone_payload_content_reader_read(
                  content_reader, (void *)buf, buf_hint)) > 0) {
        int total = 0, n = 0;

        while (total < read) {
          n = fwrite(buf, 1, read - total, fptr);
          if (n == 0) {
            exit(1);
          }
          total += n;
        }
      }

      assert(stone_payload_content_reader_is_checksum_valid(content_reader) ==
             1);

      stone_payload_content_reader_destroy(content_reader);
      free(buf);
      fflush(fptr);
      fclose(fptr);
      break;
    }
    }

    current_payload += 1;
  }

  if (num_metas + num_layouts > 0) {
    print_inspect_output(metas, num_metas, layouts, num_layouts);
  }

  if (layouts != NULL) {
    free(layouts);
  }
  if (metas != NULL) {
    free(metas);
  }
  if (indexes != NULL) {
    free(indexes);
  }
  if (attributes != NULL) {
    free(attributes);
  }
  for (int i = 0; i < header.num_payloads; i++) {
    stone_payload_destroy(payloads[i]);
  }
  if (payloads != NULL) {
    free(payloads);
  }
}

size_t read_shim(void *fptr, char *buf, size_t n) {
  return fread(buf, 1, n, fptr);
}

int64_t seek_shim(void *fptr, int64_t offset, StoneSeekFrom from) {
  fseek(fptr, offset, from);
  return ftell(fptr);
}

StoneReadVTable vtable = {
    .read = read_shim,
    .seek = seek_shim,
};

int main(int argc, char *argv[]) {
  FILE *fptr;
  StoneReader *reader;
  StoneHeaderVersion version;

  if (argc != 2) {
    printf("usage: %s <stone>\n", argv[0]);
    exit(1);
  }

  printf("\n");
  printf("Reading stone from '%s'\n\n", argv[1]);
  fptr = fopen(argv[1], "r");
  stone_read(fptr, vtable, &reader, &version);
  process_reader(reader, version);
  stone_reader_destroy(reader);
  fclose(fptr);

  return 0;
}
