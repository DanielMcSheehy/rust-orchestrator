//! Interval scheduler: fires workflows that declare `triggers.every_secs`.
//!
//! Due-times are tracked in memory and seeded lazily, so a server restart
//! waits one full interval before the first firing instead of stampeding
//! every scheduled workflow at boot.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{error, info};
use uuid::Uuid;

use crate::orchestrator::launch_run;
use crate::state::SharedState;

const TICK: Duration = Duration::from_secs(2);

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut due: HashMap<Uuid, Instant> = HashMap::new();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let workflows = match state.store.list_workflows() {
                Ok(wfs) => wfs,
                Err(e) => {
                    error!("scheduler: failed to list workflows: {e}");
                    continue;
                }
            };

            let now = Instant::now();
            let mut seen = Vec::new();
            for wf in workflows {
                let Some(every) = wf.spec.triggers.every_secs.filter(|s| *s > 0) else {
                    continue;
                };
                seen.push(wf.id);
                let interval = Duration::from_secs(every);
                match due.get(&wf.id) {
                    None => {
                        due.insert(wf.id, now + interval);
                    }
                    Some(&when) if now >= when => {
                        due.insert(wf.id, now + interval);
                        info!(workflow = %wf.spec.name, "schedule fired");
                        if let Err(e) =
                            launch_run(state.clone(), wf, Value::Null, "schedule")
                        {
                            error!("scheduler: failed to launch run: {e}");
                        }
                    }
                    Some(_) => {}
                }
            }
            // Forget deleted workflows / cleared schedules.
            due.retain(|id, _| seen.contains(id));
        }
    });
}
