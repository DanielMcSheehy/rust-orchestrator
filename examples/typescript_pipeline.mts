/**
 * Deploy and run a mixed-runtime flow from TypeScript, streaming logs live.
 *
 *   cd sdks/typescript && npm install && npm run build   (once)
 *   cargo run -p cortex-server                           (in another terminal)
 *   node examples/typescript_pipeline.mts
 */
import { CortexClient, flow, task } from "../sdks/typescript/dist/index.js";

const fetchPage = task("fetch_page", async (params) => {
  const size = params.page_size as number;
  console.log(`pretending to fetch ${size} items`);
  return { items: Array.from({ length: size }, (_, i) => ({ id: i, score: (i * 7) % 100 })) };
});

// A Python task in the middle of a TypeScript-defined flow.
const rank = task(
  "rank",
  {
    runtime: "python",
    code: [
      "def handler(params, inputs):",
      "    items = inputs['fetch_page']['items']",
      "    ranked = sorted(items, key=lambda x: -x['score'])",
      "    print(f'ranked {len(ranked)} items')",
      "    return {'top': ranked[:5]}",
      "",
    ].join("\n"),
  },
  { dependsOn: [fetchPage] },
);

const summarize = task(
  "summarize",
  async (_params, inputs) => {
    const top = (inputs.rank as { top: Array<{ id: number; score: number }> }).top;
    return { best_id: top[0].id, best_score: top[0].score };
  },
  { dependsOn: [rank] },
);

const client = new CortexClient("http://localhost:7420");
const wf = await client.deploy(
  flow("ranker", [fetchPage, rank, summarize], { params: { page_size: 200 } }),
);
console.log("deployed", wf.id);

const run = await client.trigger(wf.id);
for await (const ev of client.streamRun(run.id)) {
  if (ev.type === "log") console.log("  log:", ev.line);
  if (ev.type === "run_updated") console.log("  run:", (ev.run as { state: string }).state);
}

const { tasks } = await client.getRun(run.id);
console.log("summarize →", tasks.find((t) => t.task_id === "summarize")?.result);
