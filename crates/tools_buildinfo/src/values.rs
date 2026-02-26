// SPDX-FileCopyrightText: 2025 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

pub(crate) const VERSION: &str = env!("BUILDINFO_VERSION");

pub(crate) const BUILD_TIME: &str = env!("BUILDINFO_BUILD_TIME");

#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub(crate) const GIT_FULL_HASH: &str = env!("BUILDINFO_GIT_FULL_HASH");

#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub(crate) const GIT_SHORT_HASH: &str = env!("BUILDINFO_GIT_SHORT_HASH");

#[cfg(BUILDINFO_IS_GIT_BUILD)]
pub(crate) const GIT_SUMMARY: &str = env!("BUILDINFO_GIT_SUMMARY");
