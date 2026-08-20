"""Thin AstrBot host package for AstrEmbodiment."""

from .bridge import (
    ClosedSchemaViolation,
    GenesisManifestMismatch,
    GenesisRequired,
    GenesisUnavailable,
    NativeBridge,
    NativeCoreError,
    NativeCoreUnavailable,
    RetryWait,
    SeedDigestCollision,
    StaleCausalBase,
    StaleRevision,
    UnsupportedEventKind,
)
from .contracts import ScopeTokens, scope_json
from .coordinator import GenesisCoordinator
from .persona_genesis import (
    PersonaCompilerMalformed,
    PersonaGenesisError,
    PersonaSourceSnapshot,
    compile_with_current_chat_model,
)
from .tokens import (
    bot_token,
    event_id,
    persona_token,
    relation_token,
    session_token,
    turn_id,
)

__all__ = [
    "ClosedSchemaViolation",
    "GenesisCoordinator",
    "GenesisManifestMismatch",
    "GenesisRequired",
    "GenesisUnavailable",
    "NativeBridge",
    "NativeCoreError",
    "NativeCoreUnavailable",
    "PersonaCompilerMalformed",
    "PersonaGenesisError",
    "PersonaSourceSnapshot",
    "RetryWait",
    "ScopeTokens",
    "SeedDigestCollision",
    "StaleCausalBase",
    "StaleRevision",
    "UnsupportedEventKind",
    "bot_token",
    "compile_with_current_chat_model",
    "event_id",
    "persona_token",
    "relation_token",
    "scope_json",
    "session_token",
    "turn_id",
]
