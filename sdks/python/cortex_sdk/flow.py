"""Flow and task definition primitives.

A ``@task``-decorated function is shipped to the server as source code and
executed inside a Cortex Python worker, so it must be self-contained: do its
imports inside the function body and take ``(params, inputs)`` as arguments.
"""

from __future__ import annotations

import inspect
import textwrap
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable, Optional, Union


@dataclass
class Task:
    id: str
    code: str
    runtime: str = "python"
    depends_on: list[str] = field(default_factory=list)
    params: dict[str, Any] = field(default_factory=dict)
    timeout_secs: int = 300
    retries: int = 0
    name: Optional[str] = None

    def spec(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "runtime": self.runtime,
            "code": self.code,
            "depends_on": self.depends_on,
            "params": self.params,
            "timeout_secs": self.timeout_secs,
            "retries": self.retries,
        }


def _extract_code(fn: Callable) -> str:
    """Turn a decorated function into a self-contained worker module."""
    source = textwrap.dedent(inspect.getsource(fn))
    lines = source.splitlines()
    # Strip decorator lines (they reference cortex_sdk, absent in the worker).
    start = 0
    while start < len(lines) and lines[start].lstrip().startswith("@"):
        start += 1
    body = "\n".join(lines[start:])
    return f"{body}\n\nhandler = {fn.__name__}\n"


def _dep_id(dep: Union[str, "Task", Callable]) -> str:
    if isinstance(dep, str):
        return dep
    if isinstance(dep, Task):
        return dep.id
    cortex_task = getattr(dep, "cortex_task", None)
    if isinstance(cortex_task, Task):
        return cortex_task.id
    raise TypeError(f"cannot use {dep!r} as a dependency")


def task(
    fn: Optional[Callable] = None,
    *,
    id: Optional[str] = None,
    depends_on: Iterable[Union[str, Task, Callable]] = (),
    params: Optional[dict[str, Any]] = None,
    timeout_secs: int = 300,
    retries: int = 0,
):
    """Declare a Cortex task from a Python function.

    Usable bare (``@task``) or configured
    (``@task(depends_on=[other], retries=2)``). The decorated function keeps
    working as a normal Python callable and gains a ``.cortex_task``
    attribute holding its :class:`Task` definition.
    """

    def wrap(func: Callable) -> Callable:
        func.cortex_task = Task(
            id=id or func.__name__,
            code=_extract_code(func),
            depends_on=[_dep_id(d) for d in depends_on],
            params=params or {},
            timeout_secs=timeout_secs,
            retries=retries,
        )
        return func

    if fn is not None:  # bare @task
        return wrap(fn)
    return wrap


class Flow:
    """An ordered collection of tasks forming a DAG."""

    def __init__(
        self,
        name: str,
        *,
        description: Optional[str] = None,
        params: Optional[dict[str, Any]] = None,
        tasks: Iterable[Union[Task, Callable]] = (),
        every_secs: Optional[int] = None,
        on_ingest: Optional[str] = None,
        max_parallel_tasks: int = 8,
    ):
        self.name = name
        self.description = description
        self.params = params or {}
        self.every_secs = every_secs
        self.on_ingest = on_ingest
        self.max_parallel_tasks = max_parallel_tasks
        self.tasks: list[Task] = []
        for t in tasks:
            self.add(t)

    def add(self, t: Union[Task, Callable], **overrides: Any) -> "Flow":
        if not isinstance(t, Task):
            cortex_task = getattr(t, "cortex_task", None)
            if not isinstance(cortex_task, Task):
                raise TypeError("Flow.add expects a Task or a @task-decorated function")
            t = cortex_task
        for key, value in overrides.items():
            setattr(t, key, value)
        self.tasks.append(t)
        return self

    def spec(self) -> dict[str, Any]:
        triggers: dict[str, Any] = {}
        if self.every_secs:
            triggers["every_secs"] = self.every_secs
        if self.on_ingest:
            triggers["on_ingest"] = self.on_ingest
        return {
            "name": self.name,
            "description": self.description,
            "params": self.params,
            "tasks": [t.spec() for t in self.tasks],
            "triggers": triggers,
            "max_parallel_tasks": self.max_parallel_tasks,
        }
