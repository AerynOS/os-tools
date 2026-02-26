# SPDX-FileCopyrightText: 2024 AerynOS Developers
# SPDX-License-Identifier: MPL-2.0

# The default task is to build moss
default: moss

root-dir := justfile_directory()
build-mode := env("MODE", "onboarding")

[private]
help:
  @just --list -u

[private]
build package:
  cargo build --profile {{build-mode}} -p {{package}}

[private]
licenses:
    bash licenses.sh

# Compile boulder
boulder: (build "boulder")

# Compile moss
moss: (build "moss")

# Onboarding replacement
get-started: (build "boulder") (build "moss") (licenses)
  @echo ""
  @echo "Installing boulder and moss to {{ executable_dir() }}…"
  @mkdir -p "{{ executable_dir() }}/"
  @cp "{{ root-dir }}/target/{{ build-mode }}"/boulder "{{ executable_dir() }}/"
  @cp "{{ root-dir }}/target/{{ build-mode }}"/moss "{{ executable_dir() }}/"
  @rm -rf "{{ data_dir() }}/boulder"
  @mkdir -p "{{ data_dir() }}/boulder/licenses"
  @cp -R "{{ root-dir }}/boulder/data"/{macros,*.yaml} "{{ data_dir() }}/boulder/"
  @cp "{{ root-dir }}/license-list-data/text"/* "{{ data_dir() }}/boulder/licenses"
  @mkdir -p "{{ config_dir() }}/boulder/"
  @cp -R "{{ root-dir }}/boulder/data"/profile.d "{{ config_dir() }}/boulder/"
  @echo ""
  @echo "Listing installed files…"
  @ls -hlF "{{ executable_dir() }}"/{boulder,moss} "{{ data_dir() }}/boulder" "{{ config_dir() }}/boulder"
  @echo ""
  @echo "Checking that {{executable_dir() }} is in \$PATH…"
  @echo "{{
    if env("PATH") =~ executable_dir() {
      GREEN + '…the directory is already in \$PATH. Excellent.' + NORMAL
    } else {
      RED + '…the directory is not yet in \$PATH. Please add it.' + NORMAL
    }
  }}"
  @echo ""
  @echo "Checking the location of boulder and moss executables when executed in a shell:"
  @command -v boulder
  @command -v moss
  @echo ""
  @echo "Done."
  @echo "The Aeryn OS documentation lives at https://aerynos.dev"

# Fix code issues
fix:
  @echo "Applying clippy fixes…"
  cargo clippy --fix --allow-dirty --allow-staged --workspace -- --no-deps
  @echo "Applying cargo fmt…"
  cargo fmt --all
  @echo "Fixing typos…"
  typos -w

# Run lints
lint:
  @echo "Running clippy…"
  cargo clippy --workspace -- --no-deps
  @echo "Running cargo fmt…"
  cargo fmt --all -- --check
  @echo "Checking for typos…"
  typos

# Run tests
test: lint
  @echo "Running tests in all packages…"
  cargo test --all

# Run all DB migrations
migrate: (diesel "meta" "migration run") (diesel "layout" "migration run") (diesel "state" "migration run")
# Rerun all DB migrations
migrate-redo: (diesel "meta" "migration redo") (diesel "layout" "migration redo") (diesel "state" "migration redo")

[private]
diesel db +ARGS:
  diesel \
    --config-file {{ root-dir }}/moss/src/db/{{ db }}/diesel.toml \
    --database-url sqlite://{{ root-dir }}/moss/src/db/{{ db }}/test.db \
    {{ ARGS }}

# Run libstone example
libstone example="read" *ARGS="./test/bash-completion-2.11-1-1-x86_64.stone":
  #!/bin/bash
  output=$(mktemp)
  cargo build -p libstone --release
  clang libstone/examples/{{ example }}.c -o $output -I./libstone/src/ -lstone -L./target/release/ -Wl,-rpath,./target/release/
  if [ "$USE_VALGRIND" == "1" ]; then
    time valgrind --track-origins=yes $output {{ ARGS }};
  else
    time $output {{ ARGS }};
  fi
  rm "$output"
