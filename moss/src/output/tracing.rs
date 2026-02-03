use tracing::info;

use crate::{output, repository};

/// Tracing output
#[derive(Debug, Clone, Default)]
pub struct TracingOutput {
    _tracing: TracingState,
}

impl output::Emitter for TracingOutput {
    fn emit(&self, event: &output::InternalEvent) {
        match event {
            output::InternalEvent::RepositoryManager(event) => match event {
                repository::manager::OutputEvent::RefreshStarted { num_to_refresh } => {
                    info!(
                        target: "repository_manager",
                        num_repositories = %num_to_refresh,
                        "Refreshing repositories"
                    );
                }
                repository::manager::OutputEvent::RefreshRepoStarted(id) => {
                    info!(
                        target: "repository_manager",
                        repo_id = %id,
                        "Refreshing repository"
                    );
                }
                repository::manager::OutputEvent::RefreshRepoFinished(id) => {
                    info!(
                        target: "repository_manager",
                        repo_id = %id,
                        "Repository refreshed"
                    );
                }
                repository::manager::OutputEvent::RefreshFinished { elapsed } => {
                    info!(
                        target: "repository_manager",
                        elapsed_seconds = %elapsed.as_secs_f32(),
                        "All repositories refreshed"
                    );
                }
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TracingState {
    // spans: Arc<ArcSwap<HashMap<TypeId, tracing::Span>>>,
}
