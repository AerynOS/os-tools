-- SPDX-FileCopyrightText: 2023 AerynOS Developers
-- SPDX-License-Identifier: MPL-2.0

-- Your SQL goes here

CREATE TABLE IF NOT EXISTS state (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    type TEXT NOT NULL,
    created BIGINT NOT NULL DEFAULT (unixepoch()),
    summary TEXT NULL,
    description TEXT NULL
);

CREATE TABLE IF NOT EXISTS state_selections (
    state_id INTEGER NOT NULL,
    package_id TEXT NOT NULL,
    explicit BOOLEAN NOT NULL,
    reason TEXT NULL,
    PRIMARY KEY(state_id, package_id),
    FOREIGN KEY(state_id) REFERENCES state(id) ON DELETE CASCADE
);
