import { useEffect, useRef, useState } from "react";

import { describeTask, isActive, KIND_LABELS, type Job } from "../library/format";
import type { JobsState } from "../library/useJobs";

const RECENT = 12;

/** Lista de tarefas em segundo plano (análises e separações), no estilo da lista de downloads. */
export function TasksPanel({ jobs }: { jobs: JobsState }) {
  const [open, setOpen] = useState(false);
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const rootRef = useRef<HTMLDivElement>(null);

  const visible = jobs.jobs.filter((j) => isActive(j) || !hidden.has(j.id)).slice(0, RECENT);
  // ativos primeiro (na ordem de execução), depois os mais recentes
  const active = visible.filter(isActive).sort((a, b) => a.id - b.id);
  const finished = visible.filter((j) => !isActive(j));
  const activeCount = active.length;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const clearFinished = () => setHidden((prev) => new Set([...prev, ...finished.map((j) => j.id)]));

  return (
    <div className="tasks" ref={rootRef}>
      <button
        type="button"
        className={open ? "tasks-btn open" : "tasks-btn"}
        aria-expanded={open}
        aria-haspopup="true"
        onClick={() => setOpen((v) => !v)}
        title="Análises e separações em segundo plano"
      >
        Tarefas
        {activeCount > 0 && <span className="tasks-badge">{activeCount}</span>}
      </button>
      {open && (
        <div className="tasks-pop" role="dialog" aria-label="Tarefas em segundo plano">
          {visible.length === 0 ? (
            <p className="hint">Nada em andamento. Análises e separações aparecem aqui.</p>
          ) : (
            <>
              <ul>
                {[...active, ...finished].map((job) => (
                  <TaskRow key={job.id} job={job} onCancel={() => void jobs.cancel(job.id)} />
                ))}
              </ul>
              {finished.length > 0 && (
                <button type="button" className="link tasks-clear" onClick={clearFinished}>
                  Limpar concluídas
                </button>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

function TaskRow({ job, onCancel }: { job: Job; onCancel: () => void }) {
  const running = job.status === "running";
  const failed = job.status === "failed";
  return (
    <li className={failed ? "task failed" : "task"}>
      <div className="task-head">
        <strong title={job.song_title ?? undefined}>
          {job.song_title ?? `Música ${job.song_id}`}
        </strong>
        {isActive(job) && (
          <button type="button" className="ghost" onClick={onCancel}>
            Cancelar
          </button>
        )}
      </div>
      <span className="task-kind">{KIND_LABELS[job.kind] ?? job.kind}</span>
      {isActive(job) && (
        <progress max={1} value={running ? job.progress : undefined} aria-label="Progresso" />
      )}
      <span className="task-state">{describeTask(job)}</span>
    </li>
  );
}
