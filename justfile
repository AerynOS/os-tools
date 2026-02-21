# The default task is to build moss
default: moss

root-dir := justfile_directory()
build-mode := env_var_or_default("MODE", "onboarding")
# Keep it simple for now and make installs user-local
home := env_var("HOME")
# Hacky -- should really check for the XDG_*_DIR env vars...
xdg-config-home := home + "/.config"
xdg-data-home := home + "/.local/share"
xdg-bin-home := home + "/.local/bin"
# Read '=~' as 'contains' in the regexp sense
xdg-bin-home-in-path := if env_var("PATH") =~ xdg-bin-home { 'is already in \$PATH. Excellent.' } else { 'is not yet in \$PATH. Please add it.' }

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
  @echo "Installing boulder and moss to {{xdg-bin-home}}/ ..."
  @mkdir -p "{{xdg-bin-home}}/"
  @cp "{{root-dir}}/target/{{build-mode}}"/boulder "{{xdg-bin-home}}/"
  @cp "{{root-dir}}/target/{{build-mode}}"/moss "{{xdg-bin-home}}/"
  @rm -rf "{{xdg-data-home}}/boulder"
  @mkdir -p "{{xdg-data-home}}/boulder/licenses"
  @cp -R "{{root-dir}}/boulder/data"/{macros,*.yaml} "{{xdg-data-home}}/boulder/"
  @cp "{{root-dir}}/license-list-data/text"/* "{{xdg-data-home}}/boulder/licenses"
  @mkdir -p "{{xdg-config-home}}/boulder/"
  @cp -R "{{root-dir}}/boulder/data"/profile.d "{{xdg-config-home}}/boulder/"
  @echo ""
  @echo "Listing installed files..."
  @ls -hlF "{{xdg-bin-home}}"/boulder "{{xdg-bin-home}}"/moss "{{xdg-data-home}}/boulder"
  @echo ""
  @echo "Checking that {{xdg-bin-home}} is in \$PATH..."
  @echo "... {{xdg-bin-home}} {{xdg-bin-home-in-path}}"
  @echo ""
  @echo "Checking the location of boulder and moss executables when executed in a shell:"
  @command -v boulder
  @command -v moss
  @echo ""
  @echo "Done."
  @echo ""
  @echo "The Aeryn OS documentation lives at https://aerynos.dev"
  @echo ""

# Fix code issues
fix:
  @echo "Applying clippy fixes..."
  cargo clippy --fix --allow-dirty --allow-staged --workspace -- --no-deps
  @echo "Applying cargo fmt"
  cargo fmt --all
  @echo "Fixing typos"
  typos -w

# Run lints
lint:
  @echo "Running clippy..."
  cargo clippy --workspace -- --no-deps
  @echo "Running cargo fmt.."
  cargo fmt --all -- --check
  @echo "Checking for typos..."
  typos

# Run tests
test: lint
  @echo "Running tests in all packages"
  cargo test --all

# Run all DB migrations
migrate: (diesel "meta" "migration run") (diesel "layout" "migration run") (diesel "state" "migration run")
# Rerun all DB migrations
migrate-redo: (diesel "meta" "migration redo") (diesel "layout" "migration redo") (diesel "state" "migration redo")

[private]
diesel db +ARGS:
  diesel \
    --config-file {{root-dir}}/moss/src/db/{{db}}/diesel.toml \
    --database-url sqlite://{{root-dir}}/moss/src/db/{{db}}/test.db \
    {{ARGS}}

# Run libstone example
libstone example="read" *ARGS="./test/bash-completion-2.11-1-1-x86_64.stone":
  #!/bin/bash
  output=$(mktemp)
  cargo build -p libstone --release
  clang libstone/examples/{{example}}.c -o $output -I./libstone/src/ -lstone -L./target/release/ -Wl,-rpath,./target/release/
  if [ "$USE_VALGRIND" == "1" ];
    then time valgrind --track-origins=yes $output {{ARGS}};
    else time $output {{ARGS}};
  fi
  rm "$output"
