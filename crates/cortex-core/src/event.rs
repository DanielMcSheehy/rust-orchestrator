//! Events broadcast over the live stream (`GET /api/events`, SSE).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Run, TaskRun};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CortexEvent {
    RunUpdated {
        ts: DateTime<Utc>,
        run: Run,
    },
    TaskUpdated {
        ts: DateTime<Utc>,
        task: TaskRun,
    },
    Log {
        ts: DateTime<Utc>,
        run_id: Uuid,
        task_id: String,
        line: String,
    },
    Ingested {
        ts: DateTime<Utc>,
        dataset: String,
        records: u64,
        bytes: u64,
    },
    FunctionInvoked {
        ts: DateTime<Utc>,
        name: String,
        ok: bool,
        duration_ms: u64,
    },
}

impl CortexEvent {
    pub fn run_updated(run: Run) -> Self {
        CortexEvent::RunUpdated { ts: Utc::now(), run }
    }

    pub fn task_updated(task: TaskRun) -> Self {
        CortexEvent::TaskUpdated { ts: Utc::now(), task }
    }

    pub fn log(run_id: Uuid, task_id: impl Into<String>, line: impl Into<String>) -> Self {
        CortexEvent::Log {
            ts: Utc::now(),
            run_id,
            task_id: task_id.into(),
            line: line.into(),
        }
    }

    /// The run this event belongs to, if any — used to filter per-run SSE streams.
    pub fn run_id(&self) -> Option<Uuid> {
        match self {
            CortexEvent::RunUpdated { run, .. } => Some(run.id),
            CortexEvent::TaskUpdated { task, .. } => Some(task.run_id),
            CortexEvent::Log { run_id, .. } => Some(*run_id),
            _ => None,
        }
    }
}
