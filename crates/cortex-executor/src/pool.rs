//! Warm worker pool.
//!
//! Spawning an interpreter costs ~30ms (CPython) to ~115ms (Node with
//! TypeScript stripping) — far more than most tasks themselves. In `process`
//! isolation mode the executor therefore keeps finished workers alive and
//! feeds them subsequent jobs over the same stdin/stdout channel (the shims
//! loop over requests).
//!
//! Pool rules:
//! - one worker serves one job at a time; checkout removes it from the pool
//! - a clean job (result *or* in-workload error) checks the worker back in
//! - a timeout, protocol error, or EOF kills the worker instead
//! - workers retire after `max_jobs` jobs (bounds interpreter-level
//!   accumulation, e.g. Node's ESM module cache) and the pool keeps at most
//!   `max_idle` warm workers per runtime
//!
//! Container and microVM isolation never pool — a fresh sandbox per task is
//! the point of those modes.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use tokio::io::{BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolKind {
    Python,
    /// One Node pool serves both JavaScript and TypeScript: workers start
    /// with `--experimental-strip-types`, which is a no-op for plain JS.
    Node,
}

impl PoolKind {
    fn index(self) -> usize {
        match self {
            PoolKind::Python => 0,
            PoolKind::Node => 1,
        }
    }
}

pub(crate) struct PooledWorker {
    // Held for lifetime/kill semantics (`kill_on_drop`), not accessed directly.
    _child: Child,
    pub stdin: ChildStdin,
    pub lines: Lines<BufReader<ChildStdout>>,
    pub jobs_served: u32,
    kind: PoolKind,
}

pub(crate) struct WorkerPool {
    idle: [Mutex<Vec<PooledWorker>>; 2],
    max_idle: usize,
    max_jobs: u32,
    python_bin: String,
    node_bin: String,
    shim_dir: PathBuf,
}

impl WorkerPool {
    pub fn new(python_bin: String, node_bin: String, shim_dir: &Path) -> Self {
        let env_num = |k: &str, default: u64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        WorkerPool {
            idle: [Mutex::new(Vec::new()), Mutex::new(Vec::new())],
            max_idle: env_num("CORTEX_WORKER_MAX_IDLE", 8) as usize,
            max_jobs: env_num("CORTEX_WORKER_MAX_JOBS", 128) as u32,
            python_bin,
            node_bin,
            shim_dir: shim_dir.to_path_buf(),
        }
    }

    /// Take a warm worker, or spawn a fresh one if none are idle.
    pub fn checkout(&self, kind: PoolKind) -> std::io::Result<PooledWorker> {
        if let Some(worker) = self.idle[kind.index()].lock().expect("pool lock").pop() {
            return Ok(worker);
        }
        self.spawn(kind)
    }

    /// Return a worker after a clean job; retires it when it has served
    /// enough jobs or the pool is already full.
    pub fn checkin(&self, mut worker: PooledWorker) {
        worker.jobs_served += 1;
        if worker.jobs_served >= self.max_jobs {
            return; // dropped — kill_on_drop reaps the process
        }
        let mut idle = self.idle[worker.kind.index()].lock().expect("pool lock");
        if idle.len() < self.max_idle {
            idle.push(worker);
        }
    }

    #[cfg(test)]
    pub fn idle_count(&self, kind: PoolKind) -> usize {
        self.idle[kind.index()].lock().expect("pool lock").len()
    }

    /// Spawn a brand-new worker, bypassing the idle stack — used to retry
    /// after handing a job to a worker that died while parked.
    pub fn spawn_fresh(&self, kind: PoolKind) -> std::io::Result<PooledWorker> {
        self.spawn(kind)
    }

    fn spawn(&self, kind: PoolKind) -> std::io::Result<PooledWorker> {
        let mut cmd = match kind {
            PoolKind::Python => {
                let mut c = Command::new(&self.python_bin);
                c.arg(self.shim_dir.join("worker.py"));
                c
            }
            PoolKind::Node => {
                let mut c = Command::new(&self.node_bin);
                c.arg("--experimental-strip-types")
                    .arg("--no-warnings")
                    .arg(self.shim_dir.join("worker.mjs"));
                c
            }
        };
        // stderr is inherited: shim-level failures land in the server log.
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        use tokio::io::AsyncBufReadExt;
        let stdin = child.stdin.take().expect("stdin piped");
        let lines = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
        Ok(PooledWorker {
            _child: child,
            stdin,
            lines,
            jobs_served: 0,
            kind,
        })
    }
}
