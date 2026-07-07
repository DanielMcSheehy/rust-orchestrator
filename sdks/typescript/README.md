# @cortex/sdk (TypeScript)

TypeScript/JavaScript bindings for [Cortex](../../README.md). Zero runtime
dependencies — built on `fetch` (Node 20+, Bun, Deno, browsers).

```bash
cd sdks/typescript && npm install && npm run build
```

## Define and deploy a flow

```ts
import { CortexClient, flow, task } from "@cortex/sdk";

const extract = task("extract", async (params) => ({
  values: Array.from({ length: params.n as number }, (_, i) => i),
}));

const total = task(
  "total",
  async (_params, inputs) =>
    (inputs.extract as { values: number[] }).values.reduce((a, b) => a + b, 0),
  { dependsOn: [extract], retries: 2 },
);

const client = new CortexClient("http://localhost:7420");
const wf = await client.deploy(flow("sum-pipeline", [extract, total], { params: { n: 100 } }));
const run = await client.trigger(wf.id, { wait: true });
console.log(run.state); // "completed"
```

Handlers are serialized with `Function.prototype.toString()` and executed in
an isolated worker on the server, so they must be self-contained (no captured
variables). To ship a task in another runtime, pass raw source instead:

```ts
const crunch = task("crunch", {
  runtime: "python",
  code: "def handler(params, inputs):\n    return sum(inputs['extract']['values'])\n",
});
```

## Stream a run live

```ts
const run = await client.trigger(wf.id);
for await (const event of client.streamRun(run.id)) {
  if (event.type === "log") console.log(event.line);
}
```

## Serverless functions & ingestion

```ts
await client.createFunction({
  name: "hello",
  code: 'export const handler = (params) => `hi ${params.name}`;',
});
const { result } = await client.invoke("hello", { name: "cortex" });

await client.ingest("sensor-readings", [{ sensor: "a", v: 1 }, { sensor: "b", v: 2 }]);
```
