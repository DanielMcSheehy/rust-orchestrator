"""A heavier data pipeline: ingest a batch, then let the ingest-triggered
workflow crunch it — Python for extraction/stats, TypeScript for shaping.

    pip install -e sdks/python   (or just run from the repo root)
    cargo run -p cortex-server   (in another terminal)
    python examples/python_pipeline.py
"""
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdks" / "python"))

from cortex_sdk import CortexClient, Flow, Task, task


@task
def load_batch(params, inputs):
    """Read the freshly ingested NDJSON batch from disk (heavy data stays
    out of the API payload — the trigger hands us the file path)."""
    import json

    rows = []
    with open(params["path"]) as f:
        for line in f:
            rows.append(json.loads(line))
    print(f"loaded {len(rows)} rows from {params['dataset']}")
    return {"rows": rows}


@task(depends_on=[load_batch])
def stats(params, inputs):
    values = [r["value"] for r in inputs["load_batch"]["rows"]]
    n = len(values)
    mean = sum(values) / n
    var = sum((v - mean) ** 2 for v in values) / n
    print(f"n={n} mean={mean:.3f} var={var:.3f}")
    return {"n": n, "mean": mean, "variance": var}


# Mixing runtimes: a raw TypeScript task in a Python-defined flow.
shape_report = Task(
    id="shape_report",
    runtime="typescript",
    depends_on=["stats"],
    code=(
        "export function handler(params: any, inputs: any) {\n"
        "  const s = inputs.stats;\n"
        "  return { report: `n=${s.n} mean=${s.mean.toFixed(2)}`, ok: s.n > 0 };\n"
        "}\n"
    ),
)


def main():
    client = CortexClient("http://localhost:7420")

    flow = Flow(
        "sensor-stats",
        description="Runs automatically after every ingest into sensor-batch",
        tasks=[load_batch, stats, shape_report],
        on_ingest="sensor-batch",
    )
    wf = client.deploy(flow)
    print("deployed workflow", wf["id"])

    # Stream 5k records; this triggers the workflow automatically.
    result = client.ingest(
        "sensor-batch",
        ({"sensor": f"s{i % 16}", "value": random.gauss(20, 4)} for i in range(5000)),
    )
    run_ids = result["triggered_runs"]
    print(f"ingested {result['ingested']['records']} records; triggered {run_ids}")

    for event in client.stream_run(run_ids[0]):
        if event["type"] == "log":
            print("  log:", event["line"])
        elif event["type"] == "run_updated":
            print("  run:", event["run"]["state"])


if __name__ == "__main__":
    main()
