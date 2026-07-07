import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, timeAgo } from "../api";
import { Empty } from "../components/ui";
import type { Notebook, NotebookCell } from "../types";

const STARTER_CELLS: NotebookCell[] = [
  {
    id: "c1",
    kind: "markdown",
    code: "# New notebook\n\nMix **markdown**, executable **Python / TypeScript / JavaScript** cells, and **SQL** over your datasets — with charts in between.",
    output: null,
  },
  {
    id: "c2",
    kind: "code",
    runtime: "python",
    code: 'def handler(params, inputs):\n    print("hello from the warm worker pool")\n    return {"answer": 42}\n',
    output: null,
  },
];

export default function Notebooks() {
  const [notebooks, setNotebooks] = useState<Notebook[]>([]);
  const navigate = useNavigate();

  const refresh = useCallback(() => {
    api.get<Notebook[]>("/api/notebooks").then(setNotebooks).catch(() => {});
  }, []);

  useEffect(refresh, [refresh]);

  const create = async () => {
    const nb = await api.post<Notebook>("/api/notebooks", {
      name: `notebook-${new Date().toISOString().slice(0, 10)}`,
      cells: STARTER_CELLS,
    });
    navigate(`/notebooks/${nb.id}`);
  };

  const remove = async (nb: Notebook, ev: React.MouseEvent) => {
    ev.stopPropagation();
    if (!window.confirm(`Delete notebook "${nb.name}"?`)) return;
    await api.delete(`/api/notebooks/${nb.id}`);
    refresh();
  };

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Notebooks</h1>
          <p>Executable documents — code, SQL, charts, and notes in one place.</p>
        </div>
        <button className="btn primary" onClick={create}>
          New notebook
        </button>
      </div>
      <div className="card">
        {notebooks.length === 0 ? (
          <Empty title="No notebooks yet" hint="Create one to explore data and prototype tasks." />
        ) : (
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Cells</th>
                <th className="num">Updated</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {notebooks.map((nb) => (
                <tr key={nb.id} className="rowlink" onClick={() => navigate(`/notebooks/${nb.id}`)}>
                  <td style={{ color: "var(--ink)", fontWeight: 550 }}>{nb.name}</td>
                  <td>{Array.isArray(nb.cells) ? nb.cells.length : 0}</td>
                  <td className="num muted">{timeAgo(nb.updated_at)}</td>
                  <td className="num">
                    <button className="btn sm danger" onClick={(e) => remove(nb, e)}>
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}
