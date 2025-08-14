// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

//! Progress tracking utilities for tracing

use tracing::{Span, info, info_span};

pub fn create_progress_span(name: &str) -> Span {
    info_span!("progress", phase = name, event_type = "progress_created")
}

pub fn progress_start(phase_name: &str, total_items: usize) {
    info!(
        phase = phase_name,
        total_items = total_items,
        progress = 0.0,
        event_type = "progress_start",
        "Starting progress: {}",
        phase_name
    );
}

pub fn progress_update(current: usize, total: usize, message: &str) {
    let progress = if total > 0 { current as f32 / total as f32 } else { 0.0 };
    info!(
        progress = progress,
        current = current,
        total = total,
        message = message,
        event_type = "progress_update",
        "Progress update"
    );
}

pub fn progress_completed(phase_name: &str, duration_ms: u128, items_processed: usize) {
    info!(
        phase = phase_name,
        duration_ms = duration_ms,
        items_processed = items_processed,
        progress = 1.0,
        event_type = "progress_completed",
        "Progress completed: {}",
        phase_name
    );
}
