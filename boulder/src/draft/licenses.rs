// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use jwalk::WalkDir;
use rapidfuzz::distance::levenshtein;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use tui::Styled;

pub type Error = Box<dyn std::error::Error>;

fn collect_spdx_licenses(dir: &Path) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>), Error> {
    // Collect our spdx licenses to compare against ensuring we don't match against deprecated licenses.
    let mut purified_spdx_licenses = HashSet::new();
    let spdx_license_paths: HashSet<_> = std::fs::read_dir(dir)?
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                if !e.file_name().to_str().unwrap_or_default().contains("deprecated_") {
                    purified_spdx_licenses.insert(PathBuf::from(e.file_name()));
                    Some(e.path())
                } else {
                    None
                }
            })
        })
        .collect();

    Ok((purified_spdx_licenses, spdx_license_paths))
}

fn collect_dir_licenses(
    dir: &Path,
    spdx_list: &HashSet<PathBuf>,
) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>), Error> {
    let patterns = ["copying", "license"];

    // Match potential license files
    let mut licenses = HashSet::new();
    let mut hash_direntries = HashSet::new();

    for entry in WalkDir::new(dir).max_depth(3) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let file_name = PathBuf::from(entry.file_name());
            hash_direntries.insert(file_name);

            let file_name = entry.file_name().to_string_lossy().to_lowercase();
            if patterns.iter().any(|&pattern| file_name.contains(pattern)) {
                licenses.insert(entry.path());
            }

            // Split the spdx licence e.g. GPL-2.0-or-later -> GPL then check
            // if the file name contains the split value, if it does
            // add it to our licences to check against.
            let file_name_contains_licence: Vec<_> = spdx_list
                .par_iter()
                .filter_map(|license| match license.to_string_lossy().split_once("-") {
                    Some((key, _)) => {
                        if file_name.starts_with(&key.to_lowercase()) {
                            Some(license)
                        } else {
                            None
                        }
                    }
                    None => None,
                })
                .collect();

            if !file_name_contains_licence.is_empty() {
                licenses.insert(entry.path());
            }
        }
    }

    Ok((licenses, hash_direntries))
}

pub fn match_licences(dir: &Path, spdx_dir: &Path) -> Result<Vec<String>, Error> {
    let (spdx_pure, spdx_paths) = collect_spdx_licenses(spdx_dir)?;
    let (licenses, dir_entries) = collect_dir_licenses(dir, &spdx_pure)?;

    let reuse_matches: Vec<_> = dir_entries
        .intersection(&spdx_pure)
        .map(|m| m.with_extension("").to_str().unwrap_or_default().to_owned())
        .collect();

    if !reuse_matches.is_empty() {
        return Ok(reuse_matches);
    }

    if licenses.is_empty() {
        println!("{} | Failed to find any licenses", "Warning".yellow());
        return Ok(vec![]);
    }

    let confidence = 0.9;

    let matches: Vec<_> = licenses
        .par_iter()
        .filter_map(|license| {
            let license_content = std::fs::read_to_string(license).ok();
            if license_content.is_some() {
                Some(license_content)
            } else {
                println!("{} | Failed to parse {}", "Warning".yellow(), license.display());
                None
            }
        })
        .flat_map(|content| {
            let sanitized = content
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let scorer = levenshtein::BatchComparator::new(sanitized.chars());
            spdx_paths.par_iter().filter_map(move |spdx_license| {
                // For GNU derivate licenses SPDX includes a copy of the general GNU license below the
                // derivate license whereas downstream tarballs will typically only contain the derivate license.
                // This ruins the algorithms, just truncate to the .len() plus an additional 5% (to account for subtle
                // license variants) of the file we're comparing against to get around it.
                // NOTE: Although only reading up to n lines/chars would be quicker it has difficulty differentiating
                //       between subtle differences e.g. Apache-2.0 vs Pixar or GFDL-1.2-* vs GFDL-1.3-*.
                // TODO: How to match against multiple licences in one file? hybrid sliding window approach approach?
                let truncated_canonical: String = std::fs::read_to_string(spdx_license)
                    .ok()?
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take((sanitized.chars().count() as f64 * 1.05) as usize)
                    .collect();
                let lev_sim = scorer.normalized_similarity_with_args(
                    truncated_canonical.chars(),
                    &levenshtein::Args::default().score_cutoff(confidence),
                )?;
                if lev_sim >= confidence {
                    println!(
                        "{} | Matched against {:?} (confidence {:.2}%)",
                        "License".green(),
                        spdx_license.with_extension("").file_name().unwrap_or_default(),
                        lev_sim * 100.0
                    );
                    Some(
                        spdx_license
                            .with_extension("")
                            .file_name()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                } else {
                    None
                }
            })
        })
        .collect();

    if matches.is_empty() {
        println!("{} | Failed to match against any licenses", "Warning".yellow());
        return Ok(vec![]);
    }

    Ok(matches)
}
