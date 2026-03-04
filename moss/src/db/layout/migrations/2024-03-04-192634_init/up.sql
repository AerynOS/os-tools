-- SPDX-FileCopyrightText: 2023 AerynOS Developers
-- SPDX-License-Identifier: MPL-2.0

-- Your SQL goes here

CREATE TABLE IF NOT EXISTS layout (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    package_id TEXT NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    mode INTEGER NOT NULL,
    tag INTEGER NOT NULL,
    entry_type TEXT NOT NULL,
    entry_value1 TEXT NULL,
    entry_value2 TEXT NULL
);
