"""Direct public wrapper for the native extension bundled in a wheel."""

from __future__ import annotations

from ._native import (
    NativeCoreError,
    apply_event,
    apply_perception_proposal_v1,
    ensure_genesis,
    flush_and_close,
    health,
    inspect,
    open,
    semantic_revision_v1,
    verify_replay,
    version,
)

__all__ = [
    "NativeCoreError",
    "apply_event",
    "apply_perception_proposal_v1",
    "ensure_genesis",
    "flush_and_close",
    "health",
    "inspect",
    "open",
    "semantic_revision_v1",
    "verify_replay",
    "version",
]
