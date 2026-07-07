// Cortex Node worker shim.
//
// Reads JSON job requests from stdin, one per line:
//   {"entry": "/path/to/job.mjs", "params": {...}, "inputs": {...}}
//
// For each job it imports the ES module at `entry` (TypeScript entries rely
// on Node's type stripping), calls its exported `handler(params, inputs)`,
// and writes JSON-lines events to stdout — see worker.py for event shapes.
//
// Jobs run sequentially; the process stays alive between jobs so a warm
// worker can serve many tasks. EOF on stdin ends the process.
import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { cortex } from "./cortex.mjs";

// In-task platform bindings, available to job code as the global `cortex`.
globalThis.cortex = cortex;

const emit = (obj) => process.stdout.write(JSON.stringify(obj) + "\n");

const asLine = (args) =>
  args
    .map((a) => (typeof a === "string" ? a : JSON.stringify(a)))
    .join(" ");

for (const level of ["log", "info", "warn", "error"]) {
  console[level] = (...args) => emit({ type: "log", line: asLine(args) });
}

async function runJob(line) {
  let req;
  try {
    req = JSON.parse(line);
    const mod = await import(pathToFileURL(req.entry).href);
    const handler = mod.handler ?? mod.default;
    if (typeof handler !== "function") {
      throw new Error("job must export `handler(params, inputs)`");
    }
    const result = await handler(req.params ?? {}, req.inputs ?? {});
    emit({ type: "result", value: result ?? null });
  } catch (err) {
    emit({
      type: "error",
      message: String(err?.message ?? err),
      trace: String(err?.stack ?? ""),
    });
  }
}

// Serialize jobs: the executor sends one at a time, but handlers are async.
let queue = Promise.resolve();
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  if (line.trim()) {
    queue = queue.then(() => runJob(line));
  }
});
