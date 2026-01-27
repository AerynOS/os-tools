use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use tui::{MultiProgress, ProgressBar, ProgressStyle, Styled};

use crate::{output, repository};

/// Textual output to stdout / stderr
#[derive(Debug, Clone, Default)]
pub struct TuiOutput {
    progress: ProgressState,
}

impl output::Emitter for TuiOutput {
    fn emit(&self, event: &output::InternalEvent) {
        match event {
            output::InternalEvent::RepositoryManager(event) => self.emit_repository_manager(event),
        }
    }
}

impl TuiOutput {
    fn emit_repository_manager(&self, event: &repository::manager::OutputEvent) {
        match event {
            repository::manager::OutputEvent::RefreshStarted { .. } => {
                self.progress.multi_start();
            }
            repository::manager::OutputEvent::RefreshRepoStarted(id) => {
                let id = id.to_string();
                let pb = self.progress.multi_add_pb(
                    &id,
                    ProgressBar::new_spinner()
                        .with_style(
                            ProgressStyle::with_template(" {spinner} {wide_msg}")
                                .unwrap()
                                .tick_chars("--=≡■≡=--"),
                        )
                        .with_message(format!("{} {id}", "Refreshing".blue())),
                );
                pb.enable_steady_tick(Duration::from_millis(150));
            }
            repository::manager::OutputEvent::RefreshRepoFinished(id) => {
                let id = id.to_string();
                self.progress
                    .multi_pb_println(&id, format_args!("{} {id}", "Refreshed".green()));
                self.progress.multi_remove_pb(&id);
            }
            repository::manager::OutputEvent::RefreshFinished { .. } => {
                self.progress.multi_finish();
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProgressState {
    // ArcSwap provides lock-free safe usage in sync & async environments
    mpb: Arc<ArcSwap<MultiProgress>>,
    pbs: Arc<ArcSwap<HashMap<String, ProgressBar>>>,
}

impl ProgressState {
    fn multi_start(&self) {
        self.mpb.store(Arc::new(MultiProgress::new()));
        self.pbs.store(Arc::new(HashMap::new()));
    }

    fn multi_add_pb(&self, id: &str, pb: ProgressBar) -> ProgressBar {
        let pb = self.mpb.load().add(pb);
        self.pbs.rcu(|pbs| {
            let mut pbs = (**pbs).clone();
            pbs.insert(id.to_owned(), pb.clone());
            Arc::new(pbs)
        });
        pb
    }

    fn multi_pb_println(&self, id: &str, args: fmt::Arguments<'_>) {
        let pbs = self.pbs.load();
        pbs.get(id).expect("pb exists").suspend(|| println!("{args}"));
    }

    fn multi_remove_pb(&self, id: &str) {
        self.pbs.rcu(|pbs| {
            let mut pbs = (**pbs).clone();
            pbs.remove(id);
            Arc::new(pbs)
        });
    }

    fn multi_finish(&self) {
        self.mpb.store(Arc::new(MultiProgress::new()));
        self.pbs.store(Arc::new(HashMap::new()));
    }
}
