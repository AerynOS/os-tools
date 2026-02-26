-- SPDX-FileCopyrightText: 2023 AerynOS Developers
-- SPDX-License-Identifier: MPL-2.0

-- Your SQL goes here
CREATE TABLE IF NOT EXISTS meta (
    package TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    version_identifier TEXT NOT NULL,
    source_release INT NOT NULL,
    build_release INT NOT NULL,
    architecture TEXT NOT NULL,
    summary TEXT NOT NULL,
    description TEXT NOT NULL,
    source_id TEXT NOT NULL,
    homepage TEXT NOT NULL,
    uri TEXT NULL,
    hash TEXT NULL,
    download_size BIGINT NULL
);

CREATE TABLE IF NOT EXISTS meta_licenses (
    package TEXT NOT NULL,
    license TEXT NOT NULL,
    PRIMARY KEY (package, license),
    FOREIGN KEY (package) REFERENCES meta(package) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS meta_dependencies (
    package TEXT NOT NULL,
    dependency TEXT NOT NULL,
    PRIMARY KEY (package, dependency),
    FOREIGN KEY (package) REFERENCES meta(package) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS meta_providers (
    package TEXT NOT NULL,
    provider TEXT NOT NULL,
    PRIMARY KEY (package, provider),
    FOREIGN KEY (package) REFERENCES meta(package) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS meta_conflicts (
    package TEXT NOT NULL,
    conflict TEXT NOT NULL,
    PRIMARY KEY (package, conflict),
    FOREIGN KEY (package) REFERENCES meta(package) ON DELETE CASCADE
);
