//! Pluggable workload isolation.
//!
//! Three tiers, selected via `CORTEX_ISOLATION`:
//!
//! - `process` (default) — workers are plain OS processes under the server's
//!   user. Fastest; right for trusted single-tenant deployments.
//! - `container` — each worker runs in a fresh container (`docker`/`podman
//!   run --rm -i`) with no network, a read-only rootfs, and memory/CPU/pid
//!   limits. The shim and job directories are bind-mounted read-only.
//! - `microvm` — same container interface, but executed by a VM-backed OCI
//!   runtime (Kata Containers, Firecracker via firecracker-containerd, or
//!   any runtime class exposing hardware virtualization). Each worker gets
//!   its own kernel; the host only sees the runtime's VMM process. Requires
//!   the runtime to be installed and registered with the engine
//!   (e.g. `docker run --runtime io.containerd.kata.v2 …`).
//!
//! All tiers speak the same stdin/stdout JSON-lines protocol, so the
//! orchestrator is oblivious to the isolation level.

use std::path::Path;

use cortex_core::Runtime;

/// Paths the shim/job directories are mounted at inside containers/microVMs.
pub const GUEST_SHIM_DIR: &str = "/cortex/shim";
pub const GUEST_JOB_DIR: &str = "/cortex/job";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// Direct child processes of the server (default).
    Process { python_bin: String, node_bin: String },
    /// One container per task execution.
    Container(ContainerConfig),
    /// One microVM per task execution, via a VM-backed OCI runtime.
    MicroVm(ContainerConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerConfig {
    /// Container engine binary: `docker` or `podman`.
    pub engine: String,
    /// OCI runtime class. `None` uses the engine default; `microvm` mode
    /// requires a VM-backed runtime here (e.g. `kata`, `io.containerd.kata.v2`,
    /// `kata-fc` for the Firecracker VMM).
    pub runtime: Option<String>,
    pub python_image: String,
    pub node_image: String,
    /// Hard memory cap per worker, e.g. `512m`.
    pub memory: String,
    /// CPU quota per worker, e.g. `1` or `0.5`.
    pub cpus: String,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        ContainerConfig {
            engine: "docker".into(),
            runtime: None,
            python_image: "python:3.12-slim".into(),
            node_image: "node:22-slim".into(),
            memory: "512m".into(),
            cpus: "1".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IsolationError {
    #[error(
        "unknown CORTEX_ISOLATION `{0}` (expected `process`, `container`, or `microvm`)"
    )]
    UnknownMode(String),
}

impl Isolation {
    /// Build from environment variables. See crate docs for the full list.
    pub fn from_env() -> Result<Self, IsolationError> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let mode = get("CORTEX_ISOLATION").unwrap_or_else(|| "process".into());
        let container_config = || {
            let mut cfg = ContainerConfig {
                runtime: get("CORTEX_VM_RUNTIME"),
                ..ContainerConfig::default()
            };
            if let Some(v) = get("CORTEX_CONTAINER_ENGINE") {
                cfg.engine = v;
            }
            if let Some(v) = get("CORTEX_PYTHON_IMAGE") {
                cfg.python_image = v;
            }
            if let Some(v) = get("CORTEX_NODE_IMAGE") {
                cfg.node_image = v;
            }
            if let Some(v) = get("CORTEX_WORKER_MEMORY") {
                cfg.memory = v;
            }
            if let Some(v) = get("CORTEX_WORKER_CPUS") {
                cfg.cpus = v;
            }
            cfg
        };
        match mode.as_str() {
            "process" => Ok(Isolation::Process {
                python_bin: get("CORTEX_PYTHON_BIN").unwrap_or_else(|| "python3".into()),
                node_bin: get("CORTEX_NODE_BIN").unwrap_or_else(|| "node".into()),
            }),
            "container" => Ok(Isolation::Container(container_config())),
            "microvm" => {
                let mut cfg = container_config();
                // A microVM *is* the runtime class; default to Kata if unset.
                if cfg.runtime.is_none() {
                    cfg.runtime = Some("kata".into());
                }
                Ok(Isolation::MicroVm(cfg))
            }
            other => Err(IsolationError::UnknownMode(other.into())),
        }
    }

    pub fn mode_name(&self) -> &'static str {
        match self {
            Isolation::Process { .. } => "process",
            Isolation::Container(_) => "container",
            Isolation::MicroVm(_) => "microvm",
        }
    }
}

/// Everything needed to spawn one worker.
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    /// The job entrypoint path *as the worker will see it* — host path in
    /// process mode, guest mount path in container/microVM modes.
    pub entry: String,
    /// Container name for container/microVM modes, so a timed-out worker can
    /// be killed by name (SIGKILLing the engine client alone can orphan it).
    pub container_name: Option<String>,
}

/// Interpreter argv for a runtime, given the shim path visible to the worker.
fn interpreter_args(runtime: Runtime, python: &str, node: &str, shim_dir: &str) -> Vec<String> {
    match runtime {
        Runtime::Python => vec![python.into(), format!("{shim_dir}/worker.py")],
        Runtime::Javascript => vec![node.into(), format!("{shim_dir}/worker.mjs")],
        Runtime::Typescript => vec![
            node.into(),
            "--experimental-strip-types".into(),
            "--no-warnings".into(),
            format!("{shim_dir}/worker.mjs"),
        ],
    }
}

/// Build the launch plan for one execution.
///
/// `job_file` is the code file name inside `job_dir` (`job.py`, `job.ts`,
/// `job.mjs`); `nonce` uniquifies the container name.
pub fn plan(
    isolation: &Isolation,
    runtime: Runtime,
    shim_dir: &Path,
    job_dir: &Path,
    job_file: &str,
    nonce: &str,
) -> LaunchPlan {
    match isolation {
        Isolation::Process { python_bin, node_bin } => {
            let mut argv = interpreter_args(
                runtime,
                python_bin,
                node_bin,
                &shim_dir.to_string_lossy(),
            );
            let program = argv.remove(0);
            LaunchPlan {
                program,
                args: argv,
                entry: job_dir.join(job_file).to_string_lossy().into_owned(),
                container_name: None,
            }
        }
        Isolation::Container(cfg) | Isolation::MicroVm(cfg) => {
            let name = format!("cortex-worker-{nonce}");
            let image = match runtime {
                Runtime::Python => &cfg.python_image,
                Runtime::Typescript | Runtime::Javascript => &cfg.node_image,
            };
            let mut args: Vec<String> = vec![
                "run".into(),
                "--rm".into(),
                "-i".into(),
                "--name".into(),
                name.clone(),
                "--network".into(),
                "none".into(),
                "--read-only".into(),
                "--tmpfs".into(),
                "/tmp".into(),
                "--pids-limit".into(),
                "256".into(),
                "--memory".into(),
                cfg.memory.clone(),
                "--cpus".into(),
                cfg.cpus.clone(),
            ];
            if let Some(rt) = &cfg.runtime {
                args.push("--runtime".into());
                args.push(rt.clone());
            }
            args.push("-v".into());
            args.push(format!("{}:{GUEST_SHIM_DIR}:ro", shim_dir.to_string_lossy()));
            args.push("-v".into());
            args.push(format!("{}:{GUEST_JOB_DIR}:ro", job_dir.to_string_lossy()));
            args.push(image.clone());
            // Images ship the interpreter on PATH as python3 / node.
            args.extend(interpreter_args(runtime, "python3", "node", GUEST_SHIM_DIR));
            LaunchPlan {
                program: cfg.engine.clone(),
                args,
                entry: format!("{GUEST_JOB_DIR}/{job_file}"),
                container_name: Some(name),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dirs() -> (PathBuf, PathBuf) {
        (PathBuf::from("/host/shims"), PathBuf::from("/host/job-1"))
    }

    fn process_isolation() -> Isolation {
        Isolation::Process {
            python_bin: "python3".into(),
            node_bin: "node".into(),
        }
    }

    #[test]
    fn process_mode_runs_interpreter_directly() {
        let (shim, job) = dirs();
        let p = plan(&process_isolation(), Runtime::Python, &shim, &job, "job.py", "n1");
        assert_eq!(p.program, "python3");
        assert_eq!(p.args, vec!["/host/shims/worker.py"]);
        assert_eq!(p.entry, "/host/job-1/job.py");
        assert_eq!(p.container_name, None);
    }

    #[test]
    fn process_mode_typescript_gets_strip_types() {
        let (shim, job) = dirs();
        let p = plan(&process_isolation(), Runtime::Typescript, &shim, &job, "job.ts", "n1");
        assert_eq!(p.program, "node");
        assert_eq!(
            p.args,
            vec![
                "--experimental-strip-types",
                "--no-warnings",
                "/host/shims/worker.mjs"
            ]
        );
        assert_eq!(p.entry, "/host/job-1/job.ts");
    }

    #[test]
    fn container_mode_sandboxes_and_mounts() {
        let (shim, job) = dirs();
        let iso = Isolation::Container(ContainerConfig::default());
        let p = plan(&iso, Runtime::Python, &shim, &job, "job.py", "abc123");
        assert_eq!(p.program, "docker");
        let joined = p.args.join(" ");
        assert!(joined.starts_with("run --rm -i --name cortex-worker-abc123"));
        assert!(joined.contains("--network none"));
        assert!(joined.contains("--read-only"));
        assert!(joined.contains("--memory 512m"));
        assert!(joined.contains("--cpus 1"));
        assert!(joined.contains("-v /host/shims:/cortex/shim:ro"));
        assert!(joined.contains("-v /host/job-1:/cortex/job:ro"));
        assert!(joined.contains("python:3.12-slim python3 /cortex/shim/worker.py"));
        assert!(!joined.contains("--runtime"), "no runtime class unless configured");
        assert_eq!(p.entry, "/cortex/job/job.py");
        assert_eq!(p.container_name.as_deref(), Some("cortex-worker-abc123"));
    }

    #[test]
    fn microvm_mode_selects_vm_runtime_class() {
        let (shim, job) = dirs();
        let iso = Isolation::MicroVm(ContainerConfig {
            runtime: Some("io.containerd.kata.v2".into()),
            ..ContainerConfig::default()
        });
        let p = plan(&iso, Runtime::Typescript, &shim, &job, "job.ts", "n9");
        let joined = p.args.join(" ");
        assert!(joined.contains("--runtime io.containerd.kata.v2"));
        assert!(joined.contains(
            "node:22-slim node --experimental-strip-types --no-warnings /cortex/shim/worker.mjs"
        ));
        assert_eq!(p.entry, "/cortex/job/job.ts");
    }

    #[test]
    fn from_env_defaults_to_process() {
        // Only meaningful when the var is unset in the test environment.
        if std::env::var("CORTEX_ISOLATION").is_err() {
            assert_eq!(Isolation::from_env().unwrap().mode_name(), "process");
        }
    }
}
