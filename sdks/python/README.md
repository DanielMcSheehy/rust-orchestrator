# cortex-sdk (Python)

Python bindings for [Cortex](../../README.md). Zero dependencies — everything
runs on the standard library.

```bash
pip install -e sdks/python
```

## Define and deploy a flow

```python
from cortex_sdk import CortexClient, Flow, task

@task
def extract(params, inputs):
    print("pulling", params["n"], "records")
    return {"values": list(range(params["n"]))}

@task(depends_on=[extract], retries=2)
def total(params, inputs):
    return sum(inputs["extract"]["values"])

client = CortexClient("http://localhost:7420")
workflow = client.deploy(Flow("sum-pipeline", params={"n": 100}, tasks=[extract, total]))

run = client.trigger(workflow["id"], wait=True)
print(run["state"])                      # "completed"
```

Task functions are shipped to the server as source and executed in an
isolated worker process, so they must be self-contained: import inside the
function body, and accept `(params, inputs)` where `inputs` maps upstream
task ids to their results.

## Stream a run live

```python
run = client.trigger(workflow["id"])
for event in client.stream_run(run["id"]):
    print(event["type"], event.get("line", ""))
```

## Serverless functions

```python
client.create_function("hello", "def handler(params, inputs):\n    return f\"hi {params['name']}\"\n")
print(client.invoke("hello", {"name": "cortex"}))   # {"ok": true, "result": "hi cortex", ...}
```

## Ingest data

```python
client.ingest("sensor-readings", ({"sensor": i, "v": i * 0.5} for i in range(10_000)))
```

Flows created with `Flow(..., on_ingest="sensor-readings")` run automatically
after each ingest; `Flow(..., every_secs=300)` runs on a fixed interval.
