from __future__ import annotations

from pathlib import Path

import pytest

from astr_embodiment.relation import (
    RelationBindingError,
    canonical_relation_key,
    relation_token_from_key,
)
from astr_embodiment.secret_store import (
    SecretStoreUnavailable,
    WindowsDpapiAesGcmStore,
)


def test_relation_key_is_nfc_length_prefixed_and_stable() -> None:
    composed = canonical_relation_key(
        platform_id="平台",
        bot_id="bot",
        persona_id="é",
        target_kind="private",
        target_id="user",
    )
    decomposed = canonical_relation_key(
        platform_id="平台",
        bot_id="bot",
        persona_id="e\u0301",
        target_kind="private",
        target_id="user",
    )
    assert composed == decomposed
    assert relation_token_from_key(composed) == relation_token_from_key(decomposed)
    assert len(relation_token_from_key(composed)) == 32


def test_private_and_group_relations_never_share_identity() -> None:
    private = canonical_relation_key(
        platform_id="platform",
        bot_id="bot",
        persona_id="persona",
        target_kind="private",
        target_id="42",
    )
    group = canonical_relation_key(
        platform_id="platform",
        bot_id="bot",
        persona_id="persona",
        target_kind="group",
        target_id="42",
    )
    assert relation_token_from_key(private) != relation_token_from_key(group)
    with pytest.raises(RelationBindingError):
        canonical_relation_key(
            platform_id="platform",
            bot_id="",
            persona_id="persona",
            target_kind="private",
            target_id="42",
        )


def test_ciphertext_and_aad_tampering_are_rejected(tmp_path: Path) -> None:
    store = WindowsDpapiAesGcmStore(tmp_path)
    if not store.available:
        with pytest.raises(SecretStoreUnavailable):
            store.encrypt_target(
                umo="platform:FriendMessage:user",
                target_kind="private",
                platform_token="01" * 16,
                bot_token="02" * 16,
                persona_token="03" * 16,
                relation_token="04" * 16,
                session_token="05" * 16,
                binding_generation=1,
                bound_at_utc_ms=10,
            )
        return

    envelope = store.encrypt_target(
        umo="platform:FriendMessage:user",
        target_kind="private",
        platform_token="01" * 16,
        bot_token="02" * 16,
        persona_token="03" * 16,
        relation_token="04" * 16,
        session_token="05" * 16,
        binding_generation=1,
        bound_at_utc_ms=10,
    )
    assert store.decrypt_target(envelope) == "platform:FriendMessage:user"

    ciphertext_tampered = dict(envelope)
    ciphertext_tampered["umo_ciphertext"] = list(envelope["umo_ciphertext"])
    ciphertext_tampered["umo_ciphertext"][0] ^= 1
    with pytest.raises(RelationBindingError):
        store.decrypt_target(ciphertext_tampered)

    aad_tampered = dict(envelope)
    aad_tampered["session_token"] = "06" * 16
    with pytest.raises(RelationBindingError):
        store.decrypt_target(aad_tampered)


def test_non_windows_capability_fails_closed(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr("astr_embodiment.secret_store.platform.system", lambda: "Linux")
    store = WindowsDpapiAesGcmStore(tmp_path)
    assert not store.available
    assert not (tmp_path / "autonomy-target-key.dpapi").exists()
