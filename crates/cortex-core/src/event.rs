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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Run, RunState, TaskRun, TaskSpec, Workflow, WorkflowSpec};
    use serde_json::{json, Value};

    fn sample_workflow() -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            spec: WorkflowSpec {
                name: "wf".into(),
                description: None,
                params: Value::Null,
                tasks: vec![],
                triggers: Default::default(),
                max_parallel_tasks: 1,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_task_run(run_id: Uuid) -> TaskRun {
        let spec = TaskSpec {
            id: "t".into(),
            name: None,
            runtime: crate::model::Runtime::Python,
            code: String::new(),
            depends_on: vec![],
            params: Value::Null,
            timeout_secs: 1,
            retries: 0,
        };
        TaskRun::new(run_id, &spec)
    }

    #[test]
    fn run_updated_carries_run_and_its_id() {
        let run = Run::new(&sample_workflow(), Value::Null, "manual");
        let expected = run.id;
        let ev = CortexEvent::run_updated(run);
        assert_eq!(ev.run_id(), Some(expected));
        assert!(matches!(ev, CortexEvent::RunUpdated { .. }));
    }

    #[test]
    fn task_updated_run_id_comes_from_task() {
        let run_id = Uuid::new_v4();
        let ev = CortexEvent::task_updated(sample_task_run(run_id));
        assert_eq!(ev.run_id(), Some(run_id));
    }

    #[test]
    fn log_builder_accepts_str_and_string_and_keeps_run_id() {
        let run_id = Uuid::new_v4();
        let ev = CortexEvent::log(run_id, "task-a", String::from("hello"));
        match &ev {
            CortexEvent::Log {
                run_id: rid,
                task_id,
                line,
                ..
            } => {
                assert_eq!(*rid, run_id);
                assert_eq!(task_id, "task-a");
                assert_eq!(line, "hello");
            }
            other => panic!("expected Log, got {other:?}"),
        }
        assert_eq!(ev.run_id(), Some(run_id));
    }

    #[test]
    fn ingested_and_function_invoked_have_no_run_id() {
        let ingested = CortexEvent::Ingested {
            ts: Utc::now(),
            dataset: "ds".into(),
            records: 3,
            bytes: 42,
        };
        assert_eq!(ingested.run_id(), None);

        let invoked = CortexEvent::FunctionInvoked {
            ts: Utc::now(),
            name: "fn".into(),
            ok: true,
            duration_ms: 12,
        };
        assert_eq!(invoked.run_id(), None);
    }

    // The SSE contract: every event serializes with a snake_case `type` tag.
    #[test]
    fn serializes_with_snake_case_type_tag() {
        let run = Run::new(&sample_workflow(), Value::Null, "manual");
        let v: Value = serde_json::to_value(CortexEvent::run_updated(run)).unwrap();
        assert_eq!(v["type"], json!("run_updated"));

        let v: Value =
            serde_json::to_value(CortexEvent::task_updated(sample_task_run(Uuid::new_v4())))
                .unwrap();
        assert_eq!(v["type"], json!("task_updated"));

        let v: Value =
            serde_json::to_value(CortexEvent::log(Uuid::new_v4(), "t", "l")).unwrap();
        assert_eq!(v["type"], json!("log"));

        let ingested = CortexEvent::Ingested {
            ts: Utc::now(),
            dataset: "ds".into(),
            records: 1,
            bytes: 1,
        };
        assert_eq!(serde_json::to_value(ingested).unwrap()["type"], json!("ingested"));

        let invoked = CortexEvent::FunctionInvoked {
            ts: Utc::now(),
            name: "fn".into(),
            ok: false,
            duration_ms: 0,
        };
        assert_eq!(
            serde_json::to_value(invoked).unwrap()["type"],
            json!("function_invoked")
        );
    }

    #[test]
    fn event_roundtrips_through_json() {
        let run_id = Uuid::new_v4();
        let ev = CortexEvent::log(run_id, "task", "a line");
        let s = serde_json::to_string(&ev).unwrap();
        let back: CortexEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back.run_id(), Some(run_id));
    }

    // Guard the documented terminal-state invariant used by consumers of events.
    #[test]
    fn run_state_terminality() {
        assert!(RunState::Completed.is_terminal());
        assert!(!RunState::Running.is_terminal());
    }
}
