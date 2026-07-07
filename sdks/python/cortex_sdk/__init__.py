"""Cortex Python SDK.

Define tasks as plain Python functions, wire them into a flow, deploy the
flow to a Cortex server, and stream live run events — no dependencies
beyond the standard library.

    from cortex_sdk import CortexClient, Flow, task

    @task
    def extract(params, inputs):
        return {"values": list(range(params.get("n", 10)))}

    @task(depends_on=[extract])
    def total(params, inputs):
        return sum(inputs["extract"]["values"])

    flow = Flow("sum-pipeline", tasks=[extract, total])
    client = CortexClient("http://localhost:7420")
    workflow = client.deploy(flow)
    run = client.trigger(workflow["id"], params={"n": 100}, wait=True)
    print(run["state"])
"""

from .client import CortexClient
from .flow import Flow, Task, task

__all__ = ["CortexClient", "Flow", "Task", "task"]
__version__ = "0.1.0"
