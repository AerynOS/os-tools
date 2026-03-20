-- SPDX-FileCopyrightText: 2024 AerynOS Developers
-- SPDX-License-Identifier: MPL-2.0

-- This file should undo anything in `up.sql`
DROP TABLE IF EXISTS meta;
DROP TABLE IF EXISTS meta_licenses;
DROP TABLE IF EXISTS meta_dependencies;
DROP TABLE IF EXISTS meta_providers;
DROP TABLE IF EXISTS meta_conflicts;
