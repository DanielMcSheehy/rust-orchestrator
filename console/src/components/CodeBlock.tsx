// Read-only syntax-highlighted display (Prism — a full CodeMirror instance
// per displayed snippet would be waste) and the shared language type.
import Prism from "prismjs";
import "prismjs/components/prism-python";
import "prismjs/components/prism-typescript";
import "prismjs/components/prism-sql";
import "prismjs/components/prism-json";
import "prismjs/components/prism-markdown";
import { useMemo } from "react";

Prism.manual = true;

export type CodeLanguage =
  | "python"
  | "typescript"
  | "javascript"
  | "sql"
  | "json"
  | "markdown"
  | "plain";

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

export function highlight(code: string, language: CodeLanguage): string {
  const grammar = language === "plain" ? undefined : Prism.languages[language];
  if (!grammar) return escapeHtml(code);
  try {
    return Prism.highlight(code, grammar, language);
  } catch {
    return escapeHtml(code);
  }
}

export function CodeBlock({
  code,
  language,
  className,
}: {
  code: string;
  language: CodeLanguage;
  className?: string;
}) {
  const html = useMemo(() => highlight(code, language), [code, language]);
  return (
    <pre className={`result-json ${className ?? ""}`}>
      <code dangerouslySetInnerHTML={{ __html: html }} />
    </pre>
  );
}
