<!--
# SPDX-FileCopyrightText: 2023 AerynOS Developers
# SPDX-License-Identifier: MPL-2.0
-->

# 🛠️ OS Tools - Modern System State Management

## 📦 Core Tools

This repository provides two powerful Rust-based tools for managing `.stone` packages, the native package format for AerynOS:

- **moss**: Advanced package & system state manager with atomic transactions and content deduplication
- **boulder**: Modern package building tool with containerized builds and intelligent package splitting

## 🔧 Technical Overview

### .stone Package Format
The `.stone` format is a structured binary package format designed for modern, rolling-release systems. It features:

- Explicit versioning for seamless format changes
- `zstd` compression for optimal storage
- Content-addressable storage via `xxhash`
- Smart payload separation:
  - 📋 Metadata: Package info and licensing with strong typing
  - 🗂️ Layout: Filesystem structure definitions
  - 📑 Index: Content payload access mapping
  - 📦 Content: Deduplicated file storage

### System Architecture
- Content-addressable `/usr` with atomic updates via `renameat2`
- Private store and roots in `/.moss`
- Container-based transaction triggers
- Full USR merge compliance
- Stateless system design with clear separation of OS and local system configuration
- Quick system rollbacks through atomic operations

### Boulder Features
- YAML-based recipe format (`stone.yaml`, [`KDL`](https://kdl.dev) coming soon ❤️)
- Automatic subpackage splitting
- Automatic provider emission (e.g. `soname()`) and dependency use
- Uniform format for repos, manifests and packages
- Integrated build sandboxing (also supports rootless builds)
- Advanced compiler optimization profiles
- Support for architecture-specific tuning

**Note**: Using latest Rust via `rustup` is recommended.

## 📊 Status

 - [x] Read support for `.stone`
 - [x] Repository manipulation
 - [x] Plugin system for layered graph of dependencies
 - [x] Search support
 - [x] Transactions
 - [x] Installation support
 - [x] Removal support
 - [x] `sync` support
 - [x] Triggers
 - [x] GC / cleanups of latent states
 - [ ] System model
 - [ ] Subscriptions (named dependency paths and providers to augment the model)


## 🚀 Onboarding

```bash
# clone the AerynOS os-tools repo somewhere reasonable
mkdir -pv ~/repos/aos/
cd ~/repos/aos/
git clone https://github.com/AerynOS/os-tools.git
cd tools/

# Install a few prerequisites (this how you'd do it on AerynOS)
sudo moss it binutils glibc-devel linux-headers clang bsdtar-static cpio

# remember to add ~/.cargo/bin to your $PATH if this is how you installed rustfmt
cargo install rustfmt

cargo install typos-cli --locked

# from inside the moss clone, this will build boulder and moss
# and install them to ${HOME}/.local/bin/ by default
just get-started

# boulder and moss rely on so-called subuid and subgid support.
# IFF you do not already have this set up for your ${USER} in /etc/subuid and /etc/subuid
# you might want to do something similar to this:
sudo touch /etc/sub{uid,gid}
sudo usermod --add-subuids 1000000-1065535 --add-subgids 1000000-1065535 root
sudo usermod --add-subuids 1065536-1131071 --add-subgids 1065536-1131071 ${USER}
```

**NB:** If you want to build .stones with boulder on your _non-aeryn_ host system, you will need to specify the
location of the boulder data files (which live in ${HOME}/.local/share/boulder if you used `just get-started` like above):

```bash
alias boulder="${HOME}/.local/bin/boulder --data-dir=${HOME}/.local/share/boulder/ --config-dir=${HOME}/.config/boulder/ --moss-root=${HOME}/.cache/boulder/"
```

## 📚 Documentation

See [aerynos.dev](https://aerynos.dev/).

## 🧪 Experiment

**NB:** Remember to use the `-D aosroot/` argument to specify a root directory, otherwise moss will happily
eat your current operating system.

```bash
just get-started

# create the aosroot/ directory
mkdir -pv aosroot/

# Add the unstable repo
moss -D aosroot/ repo add unstable https://cdn.aerynos.dev/stream/unstable/x86_64/stone.index

# List packages
moss -D aosroot/ list available

# Install something
moss -D aosroot/ install systemd bash libx11-32bit
```

If you want to create systemd-nspawn roots or bootable VMs, please check out the [img-tests](https://github.com/AerynOS/img-tests) repository.


## 🤝 Contributing changes

Please ensure all tests are running locally without issue:

```bash
$ just test

# Prior to committing a change:
$ just test # includes the just lint target

# Prior to pushing anything, apply clippy fixes:
$ just fix
```

Then create a Pull Request with your changes.

## ⚖️ License

`moss` & `boulder` are available under the terms of the [MPL-2.0](https://spdx.org/licenses/MPL-2.0.html)
