//! cortex-store — embedded SQLite persistence.
//!
//! Full domain structs are stored as JSON documents alongside a handful of
//! indexed columns used for filtering, which keeps the schema stable while
//! the domain model evolves. WAL mode is enabled so readers never block the
//! orchestrator's writes.

use std::path::Path;

use chrono::Utc;
use cortex_core::{Connector, Dataset, Function, Notebook, Run, RunState, Stats, TaskRun, Workflow};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workflows (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  data       TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
  id          TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  state       TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  data        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runs_workflow ON runs (workflow_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_created  ON runs (created_at DESC);
CREATE TABLE IF NOT EXISTS task_runs (
  id     TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  data   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_runs_run ON task_runs (run_id);
CREATE TABLE IF NOT EXISTS functions (
  id   TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS datasets (
  name TEXT PRIMARY KEY,
  data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connectors (
  name TEXT PRIMARY KEY,
  data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notebooks (
  id   TEXT PRIMARY KEY,
  data TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    // ── workflows ────────────────────────────────────────────────────────

    pub fn put_workflow(&self, wf: &Workflow) -> Result<()> {
        let data = serde_json::to_string(wf)?;
        self.conn.lock().execute(
            "INSERT INTO workflows (id, name, data, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = ?2, data = ?3, updated_at = ?4",
            params![wf.id.to_string(), wf.spec.name, data, wf.updated_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_workflow(&self, id: Uuid) -> Result<Workflow> {
        let conn = self.conn.lock();
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM workflows WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        match data {
            Some(d) => Ok(serde_json::from_str(&d)?),
            None => Err(StoreError::NotFound(format!("workflow {id}"))),
        }
    }

    pub fn list_workflows(&self) -> Result<Vec<Workflow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM workflows ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_json(rows)
    }

    pub fn delete_workflow(&self, id: Uuid) -> Result<()> {
        let n = self.conn.lock().execute(
            "DELETE FROM workflows WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("workflow {id}")));
        }
        Ok(())
    }

    // ── runs ─────────────────────────────────────────────────────────────

    pub fn put_run(&self, run: &Run) -> Result<()> {
        let data = serde_json::to_string(run)?;
        self.conn.lock().execute(
            "INSERT INTO runs (id, workflow_id, state, created_at, data)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET state = ?3, data = ?5",
            params![
                run.id.to_string(),
                run.workflow_id.to_string(),
                run.state.as_str(),
                run.created_at.to_rfc3339(),
                data
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: Uuid) -> Result<Run> {
        let conn = self.conn.lock();
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM runs WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        match data {
            Some(d) => Ok(serde_json::from_str(&d)?),
            None => Err(StoreError::NotFound(format!("run {id}"))),
        }
    }

    pub fn list_runs(&self, workflow_id: Option<Uuid>, limit: u32) -> Result<Vec<Run>> {
        let conn = self.conn.lock();
        match workflow_id {
            Some(wf) => {
                let mut stmt = conn.prepare(
                    "SELECT data FROM runs WHERE workflow_id = ?1
                     ORDER BY created_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![wf.to_string(), limit], |r| r.get::<_, String>(0))?;
                collect_json(rows)
            }
            None => {
                let mut stmt =
                    conn.prepare("SELECT data FROM runs ORDER BY created_at DESC LIMIT ?1")?;
                let rows = stmt.query_map(params![limit], |r| r.get::<_, String>(0))?;
                collect_json(rows)
            }
        }
    }

    /// Most recent scheduled run for a workflow — used by the scheduler to
    /// decide whether an interval trigger is due.
    pub fn latest_run_created_at(&self, workflow_id: Uuid, trigger: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();
        let ts: Option<String> = conn
            .query_row(
                "SELECT r.created_at FROM runs r
                 WHERE r.workflow_id = ?1
                   AND json_extract(r.data, '$.trigger') = ?2
                 ORDER BY r.created_at DESC LIMIT 1",
                params![workflow_id.to_string(), trigger],
                |r| r.get(0),
            )
            .optional()?;
        Ok(ts)
    }

    // ── task runs ────────────────────────────────────────────────────────

    pub fn put_task_run(&self, task: &TaskRun) -> Result<()> {
        let data = serde_json::to_string(task)?;
        self.conn.lock().execute(
            "INSERT INTO task_runs (id, run_id, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET data = ?3",
            params![task.id.to_string(), task.run_id.to_string(), data],
        )?;
        Ok(())
    }

    pub fn list_task_runs(&self, run_id: Uuid) -> Result<Vec<TaskRun>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM task_runs WHERE run_id = ?1")?;
        let rows = stmt.query_map(params![run_id.to_string()], |r| r.get::<_, String>(0))?;
        collect_json(rows)
    }

    // ── functions ────────────────────────────────────────────────────────

    pub fn put_function(&self, f: &Function) -> Result<()> {
        let data = serde_json::to_string(f)?;
        self.conn.lock().execute(
            "INSERT INTO functions (id, name, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET data = ?3",
            params![f.id.to_string(), f.spec.name, data],
        )?;
        Ok(())
    }

    pub fn get_function(&self, name: &str) -> Result<Function> {
        let conn = self.conn.lock();
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM functions WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        match data {
            Some(d) => Ok(serde_json::from_str(&d)?),
            None => Err(StoreError::NotFound(format!("function {name}"))),
        }
    }

    pub fn list_functions(&self) -> Result<Vec<Function>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM functions ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_json(rows)
    }

    pub fn delete_function(&self, name: &str) -> Result<()> {
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM functions WHERE name = ?1", params![name])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("function {name}")));
        }
        Ok(())
    }

    // ── datasets ─────────────────────────────────────────────────────────

    /// Add records/bytes to a dataset's running totals, creating it on first ingest.
    pub fn record_ingest(&self, name: &str, records: u64, bytes: u64) -> Result<Dataset> {
        let conn = self.conn.lock();
        let existing: Option<String> = conn
            .query_row(
                "SELECT data FROM datasets WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        let now = Utc::now();
        let ds = match existing {
            Some(d) => {
                let mut ds: Dataset = serde_json::from_str(&d)?;
                ds.records += records;
                ds.bytes += bytes;
                ds.updated_at = now;
                ds
            }
            None => Dataset {
                name: name.to_string(),
                records,
                bytes,
                created_at: now,
                updated_at: now,
            },
        };
        conn.execute(
            "INSERT INTO datasets (name, data) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET data = ?2",
            params![name, serde_json::to_string(&ds)?],
        )?;
        Ok(ds)
    }

    pub fn list_datasets(&self) -> Result<Vec<Dataset>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM datasets ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_json(rows)
    }

    // ── connectors ───────────────────────────────────────────────────────

    pub fn put_connector(&self, c: &Connector) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO connectors (name, data) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET data = ?2",
            params![c.name, serde_json::to_string(c)?],
        )?;
        Ok(())
    }

    pub fn get_connector(&self, name: &str) -> Result<Connector> {
        let conn = self.conn.lock();
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM connectors WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        match data {
            Some(d) => Ok(serde_json::from_str(&d)?),
            None => Err(StoreError::NotFound(format!("connector {name}"))),
        }
    }

    pub fn list_connectors(&self) -> Result<Vec<Connector>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM connectors ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        collect_json(rows)
    }

    pub fn delete_connector(&self, name: &str) -> Result<()> {
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM connectors WHERE name = ?1", params![name])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("connector {name}")));
        }
        Ok(())
    }

    // ── notebooks ────────────────────────────────────────────────────────

    pub fn put_notebook(&self, nb: &Notebook) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO notebooks (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data = ?2",
            params![nb.id.to_string(), serde_json::to_string(nb)?],
        )?;
        Ok(())
    }

    pub fn get_notebook(&self, id: Uuid) -> Result<Notebook> {
        let conn = self.conn.lock();
        let data: Option<String> = conn
            .query_row(
                "SELECT data FROM notebooks WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        match data {
            Some(d) => Ok(serde_json::from_str(&d)?),
            None => Err(StoreError::NotFound(format!("notebook {id}"))),
        }
    }

    pub fn list_notebooks(&self) -> Result<Vec<Notebook>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM notebooks")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut nbs: Vec<Notebook> = collect_json(rows)?;
        nbs.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(nbs)
    }

    pub fn delete_notebook(&self, id: Uuid) -> Result<()> {
        let n = self.conn.lock().execute(
            "DELETE FROM notebooks WHERE id = ?1",
            params![id.to_string()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("notebook {id}")));
        }
        Ok(())
    }

    // ── stats ────────────────────────────────────────────────────────────

    pub fn stats(&self) -> Result<Stats> {
        let conn = self.conn.lock();
        let count = |sql: &str| -> rusqlite::Result<u64> {
            conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as u64)
        };
        let run_count = |state: RunState| -> rusqlite::Result<u64> {
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE state = ?1",
                params![state.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as u64)
        };
        let (records, bytes): (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(json_extract(data, '$.records')), 0),
                    COALESCE(SUM(json_extract(data, '$.bytes')), 0) FROM datasets",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Stats {
            workflows: count("SELECT COUNT(*) FROM workflows")?,
            functions: count("SELECT COUNT(*) FROM functions")?,
            datasets: count("SELECT COUNT(*) FROM datasets")?,
            runs_total: count("SELECT COUNT(*) FROM runs")?,
            runs_running: run_count(RunState::Running)? + run_count(RunState::Pending)?,
            runs_completed: run_count(RunState::Completed)?,
            runs_failed: run_count(RunState::Failed)?,
            records_ingested: records as u64,
            bytes_ingested: bytes as u64,
        })
    }
}

fn collect_json<T, I>(rows: I) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
    I: Iterator<Item = rusqlite::Result<String>>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(serde_json::from_str(&row?)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::{Runtime, TaskSpec, TriggerSpec, WorkflowSpec};
    use serde_json::json;

    fn sample_workflow() -> Workflow {
        let now = Utc::now();
        Workflow {
            id: Uuid::new_v4(),
            spec: WorkflowSpec {
                name: "etl".into(),
                description: Some("test".into()),
                params: json!({"batch": 100}),
                tasks: vec![TaskSpec {
                    id: "extract".into(),
                    name: None,
                    runtime: Runtime::Python,
                    code: "def handler(params, inputs): return 1".into(),
                    depends_on: vec![],
                    params: json!({}),
                    timeout_secs: 60,
                    retries: 0,
                }],
                triggers: TriggerSpec::default(),
                max_parallel_tasks: 8,
            },
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn workflow_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let wf = sample_workflow();
        store.put_workflow(&wf).unwrap();
        let got = store.get_workflow(wf.id).unwrap();
        assert_eq!(got.spec.name, "etl");
        assert_eq!(store.list_workflows().unwrap().len(), 1);
        store.delete_workflow(wf.id).unwrap();
        assert!(matches!(
            store.get_workflow(wf.id),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn run_lifecycle_and_stats() {
        let store = Store::open_in_memory().unwrap();
        let wf = sample_workflow();
        store.put_workflow(&wf).unwrap();

        let mut run = Run::new(&wf, json!({}), "manual");
        store.put_run(&run).unwrap();
        run.state = RunState::Completed;
        store.put_run(&run).unwrap();

        let got = store.get_run(run.id).unwrap();
        assert_eq!(got.state, RunState::Completed);

        let mut task = TaskRun::new(run.id, &wf.spec.tasks[0]);
        task.logs.push("hello".into());
        store.put_task_run(&task).unwrap();
        assert_eq!(store.list_task_runs(run.id).unwrap().len(), 1);

        store.record_ingest("events", 10, 2048).unwrap();
        store.record_ingest("events", 5, 1024).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.workflows, 1);
        assert_eq!(stats.runs_completed, 1);
        assert_eq!(stats.records_ingested, 15);
        assert_eq!(stats.bytes_ingested, 3072);
    }

    #[test]
    fn functions_unique_by_name() {
        let store = Store::open_in_memory().unwrap();
        let now = Utc::now();
        let mut f = Function {
            id: Uuid::new_v4(),
            spec: cortex_core::FunctionSpec {
                name: "resize".into(),
                description: None,
                runtime: Runtime::Javascript,
                code: "export const handler = () => 42".into(),
                timeout_secs: 30,
            },
            invocations: 0,
            created_at: now,
            updated_at: now,
        };
        store.put_function(&f).unwrap();
        f.invocations = 3;
        store.put_function(&f).unwrap();
        assert_eq!(store.get_function("resize").unwrap().invocations, 3);
        assert_eq!(store.list_functions().unwrap().len(), 1);
    }
}
