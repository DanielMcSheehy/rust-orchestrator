import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import Dashboard from "./pages/Dashboard";
import Functions from "./pages/Functions";
import Ingestion from "./pages/Ingestion";
import NotebookEditor from "./pages/NotebookEditor";
import Notebooks from "./pages/Notebooks";
import RunDetail from "./pages/RunDetail";
import Runs from "./pages/Runs";
import WorkflowDetail from "./pages/WorkflowDetail";
import Workflows from "./pages/Workflows";

const icons = {
  dashboard: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="3" y="3" width="7" height="9" rx="1.5" />
      <rect x="14" y="3" width="7" height="5" rx="1.5" />
      <rect x="14" y="12" width="7" height="9" rx="1.5" />
      <rect x="3" y="16" width="7" height="5" rx="1.5" />
    </svg>
  ),
  workflows: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <circle cx="5" cy="6" r="2.5" />
      <circle cx="19" cy="6" r="2.5" />
      <circle cx="12" cy="18" r="2.5" />
      <path d="M6.5 8 10.5 16M17.5 8 13.5 16" />
    </svg>
  ),
  runs: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <circle cx="12" cy="12" r="9" />
      <path d="M10 8.5v7l5.5-3.5z" fill="currentColor" stroke="none" />
    </svg>
  ),
  functions: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M4 17l6-5-6-5M12 19h8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  ),
  ingestion: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <ellipse cx="12" cy="5.5" rx="7" ry="2.5" />
      <path d="M5 5.5v13c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5v-13" />
      <path d="M5 12c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5" />
    </svg>
  ),
  notebooks: (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="4" y="3" width="16" height="18" rx="2" />
      <path d="M8 3v18M12 8h4M12 12h4" />
    </svg>
  ),
};

export default function App() {
  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <svg width="22" height="22" viewBox="0 0 32 32">
            <circle cx="16" cy="16" r="13" fill="none" stroke="#6d8dff" strokeWidth="3" />
            <circle cx="16" cy="16" r="5" fill="#6d8dff" />
          </svg>
          Cortex
          <small>v0.1</small>
        </div>
        <NavLink to="/" end className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.dashboard} Dashboard
        </NavLink>
        <NavLink to="/workflows" className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.workflows} Workflows
        </NavLink>
        <NavLink to="/runs" className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.runs} Runs
        </NavLink>
        <NavLink to="/notebooks" className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.notebooks} Notebooks
        </NavLink>
        <NavLink to="/functions" className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.functions} Functions
        </NavLink>
        <NavLink to="/ingestion" className={({ isActive }) => `nav-link${isActive ? " active" : ""}`}>
          {icons.ingestion} Data
        </NavLink>
        <div className="sidebar-footer">
          <span className="live-dot" /> connected to server
        </div>
      </aside>
      <main className="main">
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="/workflows" element={<Workflows />} />
          <Route path="/workflows/:id" element={<WorkflowDetail />} />
          <Route path="/runs" element={<Runs />} />
          <Route path="/runs/:id" element={<RunDetail />} />
          <Route path="/notebooks" element={<Notebooks />} />
          <Route path="/notebooks/:id" element={<NotebookEditor />} />
          <Route path="/functions" element={<Functions />} />
          <Route path="/ingestion" element={<Ingestion />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </main>
    </div>
  );
}
