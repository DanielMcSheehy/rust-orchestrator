// Public code-editing surface. The CodeMirror editor is code-split so the
// main bundle stays lean — it loads on first render of an editable field.
import { lazy, Suspense } from "react";
import type { CodeLanguage } from "./CodeBlock";

export { CodeBlock, highlight } from "./CodeBlock";
export type { CodeLanguage } from "./CodeBlock";

const Editor = lazy(() => import("./CodeMirrorEditor"));

export interface CodeEditorProps {
  value: string;
  onChange: (next: string) => void;
  language: CodeLanguage;
  minRows?: number;
  /** Invoked on Mod-Enter (run the cell / query). */
  onRun?: () => void;
  autoFocus?: boolean;
}

export default function CodeEditor(props: CodeEditorProps) {
  return (
    <Suspense
      fallback={
        <div className="cm-host" style={{ minHeight: (props.minRows ?? 3) * 20 + 16, padding: 10 }}>
          <span className="muted mono" style={{ fontSize: 12.5 }}>
            loading editor…
          </span>
        </div>
      }
    >
      <Editor {...props} />
    </Suspense>
  );
}
