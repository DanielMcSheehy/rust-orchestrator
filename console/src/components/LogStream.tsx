import { useEffect, useRef } from "react";

export interface LogLine {
  ts: string;
  tag: string;
  line: string;
}

export default function LogStream({ lines }: { lines: LogLine[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  useEffect(() => {
    const el = ref.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div
      className="logview"
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
    >
      {lines.length === 0 && <span className="muted">waiting for output…</span>}
      {lines.map((l, i) => (
        <div className="line" key={i}>
          <span className="ts">{new Date(l.ts).toLocaleTimeString()}</span>
          <span className="tag">{l.tag}</span>
          <span>{l.line}</span>
        </div>
      ))}
    </div>
  );
}
