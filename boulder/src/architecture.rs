// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use derive_more::Display;

pub const fn host() -> Architecture {
    #[cfg(target_arch = "x86_64")]
    {
        Architecture::X86_64
    }
    #[cfg(target_arch = "x86")]
    {
        Architecture::X86
    }
    #[cfg(target_arch = "aarch64")]
    {
        Architecture::Aarch64
    }
    #[cfg(target_arch = "riscv64")]
    {
        Architecture::Riscv64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Architecture {
    X86_64,
    X86,
    Aarch64,
    Riscv64,
}

impl Architecture {
    pub fn supports_emul32(&self) -> bool {
        match self {
            Architecture::X86_64 => true,
            Architecture::X86 => false,
            Architecture::Aarch64 => true,
            Architecture::Riscv64 => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display)]
pub enum BuildTarget {
    #[display("{_0}")]
    Native(Architecture),
    #[display("emul32/{_0}")]
    Emul32(Architecture),
}

impl BuildTarget {
    pub fn emul32(&self) -> bool {
        matches!(self, BuildTarget::Emul32(_))
    }

    pub fn host_architecture(&self) -> Architecture {
        match self {
            BuildTarget::Native(arch) => *arch,
            BuildTarget::Emul32(arch) => *arch,
        }
    }
}
