//! DAG validation and scheduling order for workflow task graphs.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::TaskSpec;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DagError {
    #[error("workflow has no tasks")]
    Empty,
    #[error("duplicate task id `{0}`")]
    DuplicateTask(String),
    #[error("task `{task}` depends on unknown task `{dep}`")]
    UnknownDependency { task: String, dep: String },
    #[error("task `{0}` depends on itself")]
    SelfDependency(String),
    #[error("dependency cycle involving tasks: {0:?}")]
    Cycle(Vec<String>),
}

/// Validate that the task list forms a proper DAG.
pub fn validate_dag(tasks: &[TaskSpec]) -> Result<(), DagError> {
    if tasks.is_empty() {
        return Err(DagError::Empty);
    }
    let mut ids = HashSet::new();
    for t in tasks {
        if !ids.insert(t.id.as_str()) {
            return Err(DagError::DuplicateTask(t.id.clone()));
        }
    }
    for t in tasks {
        for dep in &t.depends_on {
            if dep == &t.id {
                return Err(DagError::SelfDependency(t.id.clone()));
            }
            if !ids.contains(dep.as_str()) {
                return Err(DagError::UnknownDependency {
                    task: t.id.clone(),
                    dep: dep.clone(),
                });
            }
        }
    }
    // Kahn's algorithm; anything left over is part of a cycle.
    let layers = kahn_layers(tasks);
    let scheduled: usize = layers.iter().map(|l| l.len()).sum();
    if scheduled != tasks.len() {
        let placed: HashSet<&String> = layers.iter().flatten().collect();
        let mut cyclic: Vec<String> = tasks
            .iter()
            .map(|t| t.id.clone())
            .filter(|id| !placed.contains(id))
            .collect();
        cyclic.sort();
        return Err(DagError::Cycle(cyclic));
    }
    Ok(())
}

/// Group task ids into execution layers: every task in layer N only depends
/// on tasks in layers < N, so each layer can run fully in parallel.
///
/// Call `validate_dag` first — on a cyclic graph this silently drops the
/// cyclic tasks.
pub fn topo_layers(tasks: &[TaskSpec]) -> Vec<Vec<String>> {
    kahn_layers(tasks)
}

fn kahn_layers(tasks: &[TaskSpec]) -> Vec<Vec<String>> {
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for t in tasks {
        indegree.entry(t.id.as_str()).or_insert(0);
        for dep in &t.depends_on {
            *indegree.entry(t.id.as_str()).or_insert(0) += 1;
            dependents.entry(dep.as_str()).or_default().push(t.id.as_str());
        }
    }

    let mut frontier: VecDeque<&str> = tasks
        .iter()
        .filter(|t| indegree[t.id.as_str()] == 0)
        .map(|t| t.id.as_str())
        .collect();

    let mut layers: Vec<Vec<String>> = Vec::new();
    while !frontier.is_empty() {
        let layer: Vec<&str> = frontier.drain(..).collect();
        for &id in &layer {
            for &next in dependents.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
                let d = indegree.get_mut(next).expect("dependent tracked");
                *d -= 1;
                if *d == 0 {
                    frontier.push_back(next);
                }
            }
        }
        layers.push(layer.into_iter().map(String::from).collect());
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Runtime;

    fn task(id: &str, deps: &[&str]) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            name: None,
            runtime: Runtime::Python,
            code: String::new(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            params: serde_json::Value::Null,
            timeout_secs: 300,
            retries: 0,
        }
    }

    #[test]
    fn accepts_diamond() {
        let tasks = vec![
            task("a", &[]),
            task("b", &["a"]),
            task("c", &["a"]),
            task("d", &["b", "c"]),
        ];
        assert_eq!(validate_dag(&tasks), Ok(()));
        let layers = topo_layers(&tasks);
        assert_eq!(layers[0], vec!["a"]);
        assert_eq!(layers[2], vec!["d"]);
        let mut mid = layers[1].clone();
        mid.sort();
        assert_eq!(mid, vec!["b", "c"]);
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(validate_dag(&[]), Err(DagError::Empty));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let tasks = vec![task("a", &[]), task("a", &[])];
        assert_eq!(validate_dag(&tasks), Err(DagError::DuplicateTask("a".into())));
    }

    #[test]
    fn rejects_unknown_dependency() {
        let tasks = vec![task("a", &["ghost"])];
        assert!(matches!(
            validate_dag(&tasks),
            Err(DagError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn rejects_self_dependency() {
        let tasks = vec![task("a", &["a"])];
        assert_eq!(validate_dag(&tasks), Err(DagError::SelfDependency("a".into())));
    }

    #[test]
    fn rejects_cycle() {
        let tasks = vec![task("a", &["b"]), task("b", &["a"]), task("c", &[])];
        match validate_dag(&tasks) {
            Err(DagError::Cycle(ids)) => assert_eq!(ids, vec!["a", "b"]),
            other => panic!("expected cycle, got {other:?}"),
        }
    }
}
