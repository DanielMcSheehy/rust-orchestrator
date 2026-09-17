use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Language runtime a task or function executes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Python,
    Typescript,
    Javascript,
}

impl Runtime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Runtime::Python => "python",
            Runtime::Typescript => "typescript",
            Runtime::Javascript => "javascript",
        }
    }
}

/// A single node in a workflow DAG.
///
/// `code` is the task body: a Python module defining `def handler(params, inputs)`,
/// or an ES module exporting `handler(params, inputs)` (TypeScript type
/// annotations are stripped at load time for the `typescript` runtime).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Unique (within the workflow) task identifier, e.g. `"extract"`.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub runtime: Runtime,
    pub code: String,
    /// Task ids this task consumes results from.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Static parameters merged over the run parameters for this task.
    #[serde(default)]
    pub params: Value,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub retries: u32,
}

fn default_timeout() -> u64 {
    300
}

/// How a workflow gets kicked off besides manual triggering.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriggerSpec {
    /// Run every N seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    /// Run whenever data lands in this dataset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_ingest: Option<String>,
}

/// The user-authored definition of a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Default run parameters, overridable per trigger.
    #[serde(default)]
    pub params: Value,
    pub tasks: Vec<TaskSpec>,
    #[serde(default)]
    pub triggers: TriggerSpec,
    /// Maximum tasks executing concurrently within one run.
    #[serde(default = "default_concurrency")]
    pub max_parallel_tasks: usize,
}

fn default_concurrency() -> usize {
    8
}

/// A stored workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub spec: WorkflowSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunState::Pending => "pending",
            RunState::Running => "running",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Failed | RunState::Cancelled
        )
    }
}

/// One execution of a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub workflow_name: String,
    pub state: RunState,
    /// Effective parameters for this run.
    pub params: Value,
    /// What started the run: `manual`, `schedule`, `ingest:<dataset>`, `sdk`.
    pub trigger: String,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
}

impl Run {
    pub fn new(workflow: &Workflow, params: Value, trigger: impl Into<String>) -> Self {
        Run {
            id: Uuid::new_v4(),
            workflow_id: workflow.id,
            workflow_name: workflow.spec.name.clone(),
            state: RunState::Pending,
            params,
            trigger: trigger.into(),
            error: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }

    pub fn duration_ms(&self) -> Option<i64> {
        match (self.started_at, self.finished_at) {
            (Some(s), Some(f)) => Some((f - s).num_milliseconds()),
            _ => None,
        }
    }
}

/// One execution of a single task within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: Uuid,
    pub run_id: Uuid,
    pub task_id: String,
    pub name: String,
    pub state: RunState,
    pub attempts: u32,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
}

impl TaskRun {
    pub fn new(run_id: Uuid, task: &TaskSpec) -> Self {
        TaskRun {
            id: Uuid::new_v4(),
            run_id,
            task_id: task.id.clone(),
            name: task.name.clone().unwrap_or_else(|| task.id.clone()),
            state: RunState::Pending,
            attempts: 0,
            result: None,
            error: None,
            logs: Vec::new(),
            started_at: None,
            finished_at: None,
        }
    }
}

/// A serverless function: a named, single-shot handler invocable over HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub runtime: Runtime,
    pub code: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub id: Uuid,
    pub spec: FunctionSpec,
    pub invocations: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A named stream of ingested records (NDJSON, one record per line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    pub name: String,
    pub records: u64,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// External query engine a connector points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    /// PostgreSQL over the native protocol.
    Postgres,
    /// ClickHouse over its HTTP interface.
    Clickhouse,
    /// chDB (embedded ClickHouse) running inside the Python worker runtime.
    Chdb,
}

impl ConnectorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectorKind::Postgres => "postgres",
            ConnectorKind::Clickhouse => "clickhouse",
            ConnectorKind::Chdb => "chdb",
        }
    }
}

/// A named external data source queryable through `POST /api/query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connector {
    pub name: String,
    pub kind: ConnectorKind,
    /// Connection URL (`postgres://…`, `http://clickhouse:8123`); unused for chdb.
    #[serde(default)]
    pub url: String,
    pub created_at: DateTime<Utc>,
}

/// A notebook document. Cells are client-defined JSON — the server stores
/// and serves them; execution happens through `/api/execute` and `/api/query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notebook {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub cells: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Aggregate counters for the dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub workflows: u64,
    pub functions: u64,
    pub datasets: u64,
    pub runs_total: u64,
    pub runs_running: u64,
    pub runs_completed: u64,
    pub runs_failed: u64,
    pub records_ingested: u64,
    pub bytes_ingested: u64,
}
