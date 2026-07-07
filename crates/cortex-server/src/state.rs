use std::path::PathBuf;
use std::sync::Arc;

use cortex_core::CortexEvent;
use cortex_executor::Executor;
use cortex_store::Store;
use tokio::sync::broadcast;

pub struct AppState {
    pub store: Store,
    pub executor: Executor,
    pub events: broadcast::Sender<CortexEvent>,
    pub data_dir: PathBuf,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(store: Store, executor: Executor, data_dir: PathBuf) -> SharedState {
        let (events, _) = broadcast::channel(4096);
        Arc::new(AppState {
            store,
            executor,
            events,
            data_dir,
        })
    }

    pub fn emit(&self, event: CortexEvent) {
        // Nobody listening is fine — the stream is best-effort telemetry.
        let _ = self.events.send(event);
    }
}
