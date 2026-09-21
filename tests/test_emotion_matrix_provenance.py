import copy
import hashlib
import json
import re
import subprocess
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOT = Path(r"G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration")
SOURCE_COMMIT = "710829ae5d3bef82ce818754354272517cb28056"
TARGET_BASELINE = "e8cf2794ee45a9fad6e54125ea282446c8463ccf"
MERGE_BASE = "8c4a606e63888351b3d8854e9006b89aa6623f07"
HASH_PROFILE = "sha256/git-blob-bytes/eol-none"
PROVENANCE_FREEZE_COMMIT = "ece92b3f21c85d3ed4319283326f25f1ceb01ad0"
PROVENANCE_BLOB_OBJECT = "1747b5550aa986b55535f688c418bfe6bec8a34f"

EXPECTED_AUDITED_SOURCE_COMMITS = [
    "a036391", "dac330e", "15a2da5", "d6cfbe5", "d8bfc7c", "867eec6",
    "8209a2b", "1774023", "3984ffb", "cc725f2", "9d373c4", "ca8d719", "710829a",
]
EXPECTED_SOURCE_BLOBS = {
    "crates/ae-contracts/src/lib.rs": "4b42175c17cdf4a4e5e982bea60f4f6c63c30323c0af7969e99fc82d90f32b70",
    "crates/ae-attention/src/lib.rs": "1b7c5141f01f4f910ee60ba1933ea6645fcc410bafdef7615ab4d00591c67c52",
    "crates/ae-attention/src/r7.rs": "efaa208c170d5c950340ed6b997028b7e2f7dacfb402a95030da051c90f6bce4",
    "crates/ae-neurofield/src/lib.rs": "c0ab2ed01917564d95865065b5b51f6f86a3e9a1eaed4a4fa5cd793f41587a16",
    "crates/ae-neurofield/src/graph_development.rs": "d014151e501e55938200b2773e94a5a2f290ad09cda5c615fff5a6f9969ed00e",
    "crates/ae-neurofield/src/graph_replay.rs": "a3b7355d6f35ba04ea42c0959f1b96793026104ae504244cd20a9dc96f040ddf",
    "crates/ae-neurofield/src/structural_delta.rs": "4121208c9d6ecf9fd207616a6e95c0c65a4f73d9d24867131195ae92b40a5891",
    "crates/ae-runtime/src/semantic.rs": "63ea4b3ac75fdd5703fd2a8cd753a6d677b7eb6d8250b26f3b81e03372864649",
    "crates/ae-runtime/src/semantic_dynamics_v2.rs": "9162735da896c82bc07fbebdcc446354bbc12138efebd05e2d8cc89566f9f91d",
    "crates/ae-runtime/src/semantic_telemetry_v1.rs": "737a1ee8d8341586f46a552ea5de0e36ca41d50381e332e9092b2562c3616a7f",
    "crates/ae-runtime/src/n2_native_assembly.rs": "47031415ea7fd2966b149ecc7b9b4359b4476f74a78c051c9c315ce6ec25086f",
    "crates/ae-runtime/src/lib.rs": "930bfe25565ae221cd05a8b0d81d4a8a5318e3dbe7ec855e3dc38b290e64a0ad",
    "crates/ae-store/src/lib.rs": "a629ff3db836d855e9392eaa940808ba26153124c7a8610a654d7b6b1265b2b7",
    "crates/ae-store/src/semantic_field_attestation.rs": "05acadb0c7d5195f1497ae3673169ef79926399a86c5c5b96a46ade534564fcd",
    "crates/ae-store/src/semantic_outbox_crypto.rs": "40a0d21efaf6ce9a52e59e71eeb02a806571ffa0c67158b1891c648d461c38f8",
    "crates/ae-pyo3/src/lib.rs": "792fe403e85d7cf599a2df48ec9888fb1c47e045e251ce13274c819a079553c6",
    "astr_embodiment/bridge.py": "23afe048cc9865544bf9cda67a501c0dea769ff480017865f952d5ded7ab69e2",
    "astr_embodiment/coordinator.py": "9a236b49bd13753f7a349f541179dedbe8e235154a2b210b16319c40e5d68577",
    "astr_embodiment/semantic_outbox.py": "92cdb95127f0bb9074e10aea1b3192964695f71a5ded83fd85ac45c4a27d7014",
    "model/regions-v1.toml": "6a2af87fe65e01b17a512b9796bee011e6ea319b1362467d426fb685e887383a",
}
REQUIRED_CAPABILITY_IDS = {
    "formula-contract", "module-export-surface", "graph-capacity-and-digests", "n2-no-action-boundary",
}
ALLOWED_DISPOSITIONS = {"EXACT_PORT", "ADAPTED_WITH_EQUIVALENCE", "EXPLICITLY_SUPERSEDED", "UNMAPPED"}
SOURCE_REF_KEYS = {"path", "kind", "qualified_name", "commit", "blob"}
TARGET_REF_KEYS = {"path", "kind", "qualified_name", "commit"}
EVIDENCE_KEYS = {"kind", "locator", "sha256", "target_commit"}
FULL_OBJECT_ID = re.compile(r"[0-9a-f]{40}(?:[0-9a-f]{24})?")
SHA256 = re.compile(r"[0-9a-f]{64}")
ROUTE_TARGET_COMMIT = "4bda9664b70da29ab36f222bb3ea32c79c2763c0"
GRAPH_DEVELOPMENT_TARGET_COMMIT = "7f1624518b07626bd68525d930b79417eb27f025"
GRAPH_ADAPTED_TARGET_COMMIT = "eb3f4d75667b72003ea391286d5104949eabed73"
DYNAMICS_TARGET_COMMIT = "626fe3026c3830b6728255328037c7312e87e101"
SEMANTIC_MIGRATION_TARGET_COMMIT = "5541db9d7c5b86b21f013bed2fa4f6c69029bdd1"
ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT = "a96e3db8b8e36df6bbe18243f21d45117727ea4f"
PAIRED_SEMANTIC_TARGET_COMMIT = "f05734077b6af7c7a397bb3875968254c485f9d8"
TASK6_FIXTURE_TARGET_COMMIT = "48150de07d52ce50674518189ca52ac10462504e"
TASK7_TARGET_COMMIT = "eb09cbf4aabf1a13cfebec85024d07d045de59cf"
TASK7_EXTRACTION_COMMIT = "616c9839b20fa000782be13eab0e8caebbfe92f8"
TASK7_EXTRACTION_PARENT = "1202e1f95daecd6a9f1c5761fec1207b92eb4201"
SUPERSEDED_TASK5_EVIDENCE_COMMIT = "c341c2800d8a2f1ca7c6da4c50893ccdd8e49aca"
RECEIPT_SCHEMA = "ae.local-reproducible-execution-receipt.v1"
TASK2_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task2-v1/"
TASK3_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task3-v1/"
TASK4_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task4-v1/"
TASK5_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task5-v1/"
TASK5_V2_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task5-v2/"
TASK6_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task6-v1/"
TASK7_RECEIPT_EVIDENCE_ROOT = "model/evidence/emotion-matrix-task7-v1/"
RECEIPT_EVIDENCE_ROOTS = (
    TASK2_RECEIPT_EVIDENCE_ROOT,
    TASK3_RECEIPT_EVIDENCE_ROOT,
    TASK4_RECEIPT_EVIDENCE_ROOT,
    TASK5_RECEIPT_EVIDENCE_ROOT,
    TASK5_V2_RECEIPT_EVIDENCE_ROOT,
    TASK6_RECEIPT_EVIDENCE_ROOT,
    TASK7_RECEIPT_EVIDENCE_ROOT,
)
RECEIPT_CWD = r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-worktree"
TASK2_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-target"
)
TASK3_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-task03-target"
)
TASK4_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-task04-quality-target"
)
TASK5_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-task05-quality-target"
)
TASK5_V2_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-task05-v2-target"
)
TASK6_RECEIPT_TARGET_DIR = (
    r"G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-task06-evidence-target"
)
TASK7_RECEIPT_TARGET_DIR = r"D:\CodexTemp\ae-t7-evidence"
EXPECTED_TOOLCHAIN = {
    "cargo": "cargo 1.97.0 (c980f4866 2026-06-30)",
    "rustc": "rustc 1.97.0 (2d8144b78 2026-07-07)",
    "rustc_commit_hash": "2d8144b7880597b6e6d3dfd63a9a9efae3f533d3",
    "host": "x86_64-pc-windows-msvc",
    "llvm": "22.1.6",
}
EXPECTED_MAPPED_CAPABILITY_IDS = {
    "evidence-slot-order",
    "evidence-range-validation",
    "frozen-15-routes",
    "route-digest",
    "canonical-262144-edge-graph",
    "graph-replay",
    "structural-delta-cas",
    "fxp6-arithmetic",
    "jacobi-before-state",
    "eight-dof-state",
    "energy-telemetry",
    "capacity-telemetry",
    "renormalization-residual-telemetry",
    "semantic-vector-receipt-v2",
    "node-observability-v2",
    "expression-projection-v1",
    "aesem2-read-compatibility",
    "aesem3-current-write",
    "finite-domain-migration",
    "migration-preimage-backup",
    "formula-upgrade-proof",
    "paired-atomic-commit",
    "event-deduplication",
    "stale-base-rejection",
    "identity-conflict-rejection",
    "evidence-canonical-codec",
    "independent-semantic-cursor",
    "relation-private-evidence",
    "persona-global-mood",
}
TASK5_REINSTATED_CAPABILITY_IDS = {
    "aesem2-read-compatibility",
    "aesem3-current-write",
    "finite-domain-migration",
    "migration-preimage-backup",
    "formula-upgrade-proof",
}
EXPECTED_CAPABILITY_DISPOSITIONS = {
    "evidence-slot-order": "EXACT_PORT",
    "evidence-range-validation": "EXACT_PORT",
    "frozen-15-routes": "EXACT_PORT",
    "route-digest": "EXACT_PORT",
    "canonical-262144-edge-graph": "EXACT_PORT",
    "graph-replay": "ADAPTED_WITH_EQUIVALENCE",
    "structural-delta-cas": "ADAPTED_WITH_EQUIVALENCE",
    "fxp6-arithmetic": "EXACT_PORT",
    "jacobi-before-state": "EXACT_PORT",
    "eight-dof-state": "EXACT_PORT",
    "energy-telemetry": "EXACT_PORT",
    "capacity-telemetry": "EXACT_PORT",
    "renormalization-residual-telemetry": "EXACT_PORT",
    "semantic-vector-receipt-v2": "ADAPTED_WITH_EQUIVALENCE",
    "node-observability-v2": "EXACT_PORT",
    "expression-projection-v1": "EXACT_PORT",
    "aesem2-read-compatibility": "ADAPTED_WITH_EQUIVALENCE",
    "aesem3-current-write": "ADAPTED_WITH_EQUIVALENCE",
    "finite-domain-migration": "ADAPTED_WITH_EQUIVALENCE",
    "migration-preimage-backup": "ADAPTED_WITH_EQUIVALENCE",
    "formula-upgrade-proof": "ADAPTED_WITH_EQUIVALENCE",
    "paired-atomic-commit": "ADAPTED_WITH_EQUIVALENCE",
    "event-deduplication": "ADAPTED_WITH_EQUIVALENCE",
    "stale-base-rejection": "EXACT_PORT",
    "identity-conflict-rejection": "ADAPTED_WITH_EQUIVALENCE",
    "evidence-canonical-codec": "EXACT_PORT",
    "independent-semantic-cursor": "ADAPTED_WITH_EQUIVALENCE",
    "relation-private-evidence": "ADAPTED_WITH_EQUIVALENCE",
    "persona-global-mood": "ADAPTED_WITH_EQUIVALENCE",
}
CONTRACT_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK2_RECEIPT_EVIDENCE_ROOT}contracts-exact.receipt.json",
    "sha256": "56a2ef0fb83aa0da9d76151642fd61a116a36266948f5314b4fc8b88b5f43472",
    "target_commit": ROUTE_TARGET_COMMIT,
}
ATTENTION_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK2_RECEIPT_EVIDENCE_ROOT}attention-exact.receipt.json",
    "sha256": "c670f071eac04176303e23fc4a99db2e2a57a9b9565585d9e5f8b7eaf69fc2cb",
    "target_commit": ROUTE_TARGET_COMMIT,
}
GRAPH_DEVELOPMENT_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK3_RECEIPT_EVIDENCE_ROOT}graph-development-exact.receipt.json",
    "sha256": "163be9916f8a82d4a22381796f6aa39071dbd12fb1bab01953bba04563f8009a",
    "target_commit": GRAPH_DEVELOPMENT_TARGET_COMMIT,
}
GRAPH_REPLAY_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK3_RECEIPT_EVIDENCE_ROOT}graph-replay-exact.receipt.json",
    "sha256": "1614e13e472ec59b2a098b60a5c91ae9db0d4ddc03718b6bfbe4844136b39958",
    "target_commit": GRAPH_ADAPTED_TARGET_COMMIT,
}
STRUCTURAL_DELTA_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK3_RECEIPT_EVIDENCE_ROOT}structural-delta-exact.receipt.json",
    "sha256": "8a65bd3e580323c2ff5950ea7b802d26927c70d56776a43bbe590d3ec785c781",
    "target_commit": GRAPH_ADAPTED_TARGET_COMMIT,
}
DYNAMICS_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK4_RECEIPT_EVIDENCE_ROOT}dynamics-exact.receipt.json",
    "sha256": "e1533335710752bae262fdce61346db383e699596714671a8751b27f3c416e54",
    "target_commit": DYNAMICS_TARGET_COMMIT,
}
TELEMETRY_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK4_RECEIPT_EVIDENCE_ROOT}telemetry-exact.receipt.json",
    "sha256": "a1a856f4a91c648bcd52e72baffe0eb9e73bfff5236ebddb04e5ba6b29631571",
    "target_commit": DYNAMICS_TARGET_COMMIT,
}
ERROR_ATOMICITY_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK4_RECEIPT_EVIDENCE_ROOT}error-atomicity-exact.receipt.json",
    "sha256": "03dceaaedb06bc6e4b331f31db7a17e2b6aaf04dc50ba8464f53ad401ac065cd",
    "target_commit": DYNAMICS_TARGET_COMMIT,
}
RUNTIME_CHECK_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK4_RECEIPT_EVIDENCE_ROOT}runtime-check.receipt.json",
    "sha256": "9f6d2dd3a0a94f5cef7dea4b672b266a473ff3076ce366571f51885f0f376b2b",
    "target_commit": DYNAMICS_TARGET_COMMIT,
}
TASK5_RUNTIME_SEMANTIC_RECEIPT = {
    "kind": "local_reproducible_execution_receipt",
    "locator": f"{TASK5_RECEIPT_EVIDENCE_ROOT}runtime-semantic-exact.receipt.json",
    "sha256": "4c4882af4c90daf943eac64e5e933996ccc3ab2d7a0c33ac58c99215ea3fef43",
    "target_commit": SEMANTIC_MIGRATION_TARGET_COMMIT,
}
def _task5_v2_receipt(name: str, digest: str) -> dict:
    return {
        "kind": "local_reproducible_execution_receipt",
        "locator": f"{TASK5_V2_RECEIPT_EVIDENCE_ROOT}{name}.receipt.json",
        "sha256": digest,
        "target_commit": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    }


TASK5_AESEM2_RECEIPT = _task5_v2_receipt(
    "runtime-aesem2-bounded",
    "0d6848a8d764ca222205672ed60844b76f9fc518934a1e7d9400ae5aa69bc0ca",
)
TASK5_AESEM3_RECEIPT = _task5_v2_receipt(
    "runtime-aesem3-four-block",
    "0dd342e4fd5276fcf6536c9620fcdab6981c76ba95eccb4fe18d387201265c32",
)
TASK5_CURRENT_WRITE_RECEIPT = _task5_v2_receipt(
    "store-finite-copy-verify-commit",
    "3fd2bfeb39dccbc05504fb824b7812275d312781b35c6369fd55dda49709c029",
)
TASK5_FINITE_RECEIPT = _task5_v2_receipt(
    "store-authority-closure",
    "084e61d082331f5497933480850e57e67ed9d7b0bb7f616108b181e70e33b58c",
)
TASK5_FORMULA_RECEIPT = _task5_v2_receipt(
    "store-formula-authority",
    "d676d71debd621272ee468d36378a3389c6af65d358a250abcd65f36de9722ec",
)
TASK5_SQL_TYPE_RECEIPT = _task5_v2_receipt(
    "store-sql-type-gates",
    "7cd4c55cf41c4f6f4370d3d5104143c8923ae9e9a07424f488eb412bc27dccb4",
)
TASK5_SQL_SIZE_RECEIPT = _task5_v2_receipt(
    "store-sql-size-gate",
    "102b06217e8e326b4623dfc5b4211720bb652f77707f05b7b7d02f17f9d84218",
)
TASK5_CREATOR_TYPE_RECEIPT = _task5_v2_receipt(
    "store-creator-type-gate",
    "6ba2c8f8e7a2596988acc915e1f5f1dcb780dfca8541180210e0e34f5e1b0fce",
)
TASK5_BACKUP_RECEIPT = _task5_v2_receipt(
    "store-backup-18-seam",
    "eac42f9726fa7c7b75c36a66d1df1cf20a33ff1fa2389484892e7da73a818bbb",
)
TASK5_ROWLESS_RECEIPT = _task5_v2_receipt(
    "store-rowless-cross-version-recovery",
    "e5e4d31a30a0f8d2fcda209e2a9b53a6f72db0066528b9fa58c8aeda577efc5c",
)
TASK5_STORE_PACKAGE_RECEIPT = _task5_v2_receipt(
    "store-anchored-package",
    "34ba6232411bad828ec19674e062b72d9376f5a10f9e1d9df230f66c9bc60e11",
)
TASK5_PLATFORM_PACKAGE_RECEIPT = _task5_v2_receipt(
    "platform-anchored-package",
    "f3343bb5c9bbb05c40b4073ae41b68cc2a12e14fd616f673f70c41788e70bed9",
)
TASK5_CAS_RECEIPT = _task5_v2_receipt(
    "store-cas-atomicity",
    "3a1fdf7da1d7a17830d3dadeb663ea91207df26fc97fd00c26cd7828323e477e",
)
TASK5_SQL_ROLLBACK_RECEIPT = _task5_v2_receipt(
    "store-sql-rollback",
    "93e80d38dee5fcc38d8c2b2349cd79f6f3c194cd3e497c961bca31a041dd447e",
)
TASK5_REOPEN_RECEIPT = _task5_v2_receipt(
    "store-reopen-reattest",
    "1c935fffe19dae6cc62cdedeb29368179177f0399870149690b54dbd4888dc9b",
)


def _task6_receipt(name: str, digest: str, target_commit: str = PAIRED_SEMANTIC_TARGET_COMMIT) -> dict:
    return {
        "kind": "local_reproducible_execution_receipt",
        "locator": f"{TASK6_RECEIPT_EVIDENCE_ROOT}{name}.receipt.json",
        "sha256": digest,
        "target_commit": target_commit,
    }


TASK6_ATOMICITY_RECEIPT = _task6_receipt(
    "store-atomicity",
    "fae924ccbb23ed96e9bfc64bd8bc09482d9166ba9bb2b2b5db37e492f580f990",
)
TASK6_DEDUPE_RECEIPT = _task6_receipt(
    "store-dedupe-stale-conflict",
    "7f69b5652b56ce0836ba4f26e973dfee57553cde94f9feb052ede03ce9aa2a9d",
)


def _task7_receipt(name: str, digest: str) -> dict:
    return {
        "kind": "local_reproducible_execution_receipt",
        "locator": f"{TASK7_RECEIPT_EVIDENCE_ROOT}{name}.receipt.json",
        "sha256": digest,
        "target_commit": TASK7_TARGET_COMMIT,
    }


TASK7_CORE_RECEIPT = _task7_receipt(
    "semantic-core-transition",
    "ce17394c7d36f21623890f82b51f8c80070ab5682e34e6c7b533dbdb302002c2",
)
TASK7_EVOLUTION_RECEIPT = _task7_receipt(
    "runtime-evolution-reopen",
    "e1f4fbf15fe584c8ec0be33edb96bf917ead6d1e401f5147415526e5d8d80baa",
)
TASK7_RELATION_RECEIPT = _task7_receipt(
    "runtime-relation-private-persona-global",
    "43a59bf6637805bcdef27cfac6588997436d7bc5660b8e67736dff9644718258",
)
TASK7_INVALID_RECEIPT = _task7_receipt(
    "runtime-invalid-stale-atomic",
    "dc780c67d1aefbeda11e0d2de00029d885e875345a07c9313d8811b3aa4b2f8e",
)
TASK7_STORE_RECEIPT = _task7_receipt(
    "store-dedupe-stale-identity",
    "8b879a18e26415fba532d0d35ea609faa90e2a746261fa7573f2df8f6dc04038",
)
TASK7_HOST_RECEIPT = _task7_receipt(
    "host-origin-authority-retry",
    "0e31a8bb7bb250646269535c217b288084f0544fc684d482c9891d21026db224",
)
TASK7_NONHOST_RECEIPT = _task7_receipt(
    "host-nonhost-rejection",
    "e31b85158268c245bf099823830dfee7c25c7744a173e33b571c6bb3703d6399",
)
DYNAMICS_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics --jobs 1 "
    "full_16k_two_edge_jacobi_step_matches_exact_dynamics_goldens -- --exact"
)
TELEMETRY_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --lib --jobs 1 "
    "semantic_telemetry_v1::tests::exact_native_telemetry_sealing_is_crate_private -- --exact"
)
ERROR_ATOMICITY_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics --jobs 1 "
    "overflow_and_invalid_shape_fail_without_partial_field -- --exact"
)
RUNTIME_CHECK_COMMAND = "cargo check --locked --offline -p ae-runtime --jobs 1"
TASK5_RUNTIME_SEMANTIC_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --lib --jobs 1 "
    "semantic::tests::aesem2_is_authenticated_read_only_and_aesem3_is_current_write -- --exact"
)
TASK5_AESEM2_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --lib --jobs 1 "
    "semantic::tests::aesem2_history_codec_is_bounded_and_authority_authenticated -- --exact"
)
TASK5_AESEM3_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --lib --jobs 1 "
    "semantic::tests::aesem3_decoder_requires_exactly_four_bounded_blocks -- --exact"
)
TASK5_CURRENT_WRITE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "finite_aesem2_migration_is_copy_verify_commit_and_idempotent -- --exact"
)
TASK5_FINITE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "authority_constructs_finite_migration_and_closes_all_history_sets -- --exact"
)
TASK5_BACKUP_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 "
    "--features migration-test-hooks --jobs 1 "
    "backup_recovers_every_failpoint_without_source_mutation -- --exact --nocapture"
)
TASK5_FORMULA_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "authority_binds_formula_upgrade_and_rejects_public_raw_envelopes -- --exact"
)
TASK5_SQL_TYPE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "task5_sqlite_type_gates_reject_dynamic_types_without_raw_driver_errors -- --exact"
)
TASK5_SQL_SIZE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "task5_oversized_authority_digest_is_rejected_by_length_before_blob_read -- --exact"
)
TASK5_CREATOR_TYPE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "task5_creator_identity_type_is_rejected_before_cast_materialization -- --exact"
)
TASK5_ROWLESS_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 "
    "--features migration-test-hooks --jobs 1 "
    "rowless_published_backup_accepts_its_historical_creator_but_not_tampering -- --exact"
)
TASK5_STORE_PACKAGE_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "backup_package_requires_exact_real_regular_file_members -- --exact"
)
TASK5_PLATFORM_PACKAGE_COMMAND = (
    "cargo test --locked --offline -p ae-platform-fs --lib --jobs 1 "
    "tests::anchored_package_opens_only_the_exact_regular_members -- --exact"
)
TASK5_CAS_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "concurrent_cas_and_exact_retry_are_zero_write_or_identical -- --exact"
)
TASK5_SQL_ROLLBACK_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 "
    "--features migration-test-hooks --jobs 1 "
    "sql_failpoints_roll_back_all_migration_rows -- --exact"
)
TASK5_REOPEN_COMMAND = (
    "cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 "
    "exact_retry_and_reopen_reattest_complete_committed_closure -- --exact"
)
TASK6_ATOMICITY_COMMAND = (
    "cargo test --locked --offline -p ae-store --lib --jobs 1 "
    "semantic_atomic_tests::canonical_event_and_semantic_sidecar_commit_together_or_not_at_all -- --exact"
)
TASK6_DEDUPE_COMMAND = (
    "cargo test --locked --offline -p ae-store --lib --jobs 1 "
    "semantic_atomic_tests::duplicate_stale_and_identity_conflict_never_split_cursors -- --exact"
)
TASK7_CORE_COMMAND = (
    "cargo test --locked --offline -p ae-semantic-core --test user_stimulus_transition "
    "--jobs 1 pure_transition_preserves_the_frozen_dynamics_and_wire_closure -- --exact"
)
TASK7_EVOLUTION_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane --jobs 1 "
    "authenticated_user_stimulus_changes_and_reopens_persona_emotion_state -- --exact"
)
TASK7_RELATION_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane --jobs 1 "
    "relation_evidence_is_private_while_persona_mood_is_continuous -- --exact"
)
TASK7_INVALID_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane --jobs 1 "
    "invalid_evidence_and_commit_fault_leave_hot_and_store_unchanged -- --exact"
)
TASK7_STORE_COMMAND = (
    "cargo test --locked --offline -p ae-store --lib --jobs 1 "
    "semantic_atomic_tests::duplicate_stale_and_identity_conflict_never_split_cursors -- --exact"
)
TASK7_HOST_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test perception_origin_authority --jobs 1 "
    "committed_host_inbound_is_the_only_mint_authority_and_retry_is_exact -- --exact"
)
TASK7_NONHOST_COMMAND = (
    "cargo test --locked --offline -p ae-runtime --test perception_origin_authority --jobs 1 "
    "wrong_origin_nonce_scope_and_non_host_source_write_no_semantics -- --exact"
)
EXPECTED_CAPABILITY_RECEIPTS = {
    "evidence-slot-order": CONTRACT_RECEIPT,
    "evidence-range-validation": CONTRACT_RECEIPT,
    "frozen-15-routes": ATTENTION_RECEIPT,
    "route-digest": CONTRACT_RECEIPT,
    "canonical-262144-edge-graph": GRAPH_DEVELOPMENT_RECEIPT,
    "graph-replay": GRAPH_REPLAY_RECEIPT,
    "structural-delta-cas": STRUCTURAL_DELTA_RECEIPT,
    "fxp6-arithmetic": DYNAMICS_RECEIPT,
    "jacobi-before-state": DYNAMICS_RECEIPT,
    "eight-dof-state": DYNAMICS_RECEIPT,
    "energy-telemetry": TELEMETRY_RECEIPT,
    "capacity-telemetry": TELEMETRY_RECEIPT,
    "renormalization-residual-telemetry": TELEMETRY_RECEIPT,
    "semantic-vector-receipt-v2": TASK5_RUNTIME_SEMANTIC_RECEIPT,
    "node-observability-v2": TASK5_RUNTIME_SEMANTIC_RECEIPT,
    "expression-projection-v1": TASK5_RUNTIME_SEMANTIC_RECEIPT,
    "aesem2-read-compatibility": TASK5_AESEM2_RECEIPT,
    "aesem3-current-write": TASK5_CURRENT_WRITE_RECEIPT,
    "finite-domain-migration": TASK5_FINITE_RECEIPT,
    "migration-preimage-backup": TASK5_BACKUP_RECEIPT,
    "formula-upgrade-proof": TASK5_FORMULA_RECEIPT,
    "paired-atomic-commit": TASK6_ATOMICITY_RECEIPT,
    "event-deduplication": TASK6_DEDUPE_RECEIPT,
    "stale-base-rejection": TASK6_DEDUPE_RECEIPT,
    "identity-conflict-rejection": TASK6_DEDUPE_RECEIPT,
    "evidence-canonical-codec": TASK7_EVOLUTION_RECEIPT,
    "independent-semantic-cursor": TASK7_EVOLUTION_RECEIPT,
    "relation-private-evidence": TASK7_RELATION_RECEIPT,
    "persona-global-mood": TASK7_RELATION_RECEIPT,
}
EXPECTED_CAPABILITY_TARGET_COMMITS = {
    "evidence-slot-order": ROUTE_TARGET_COMMIT,
    "evidence-range-validation": ROUTE_TARGET_COMMIT,
    "frozen-15-routes": ROUTE_TARGET_COMMIT,
    "route-digest": ROUTE_TARGET_COMMIT,
    "canonical-262144-edge-graph": GRAPH_DEVELOPMENT_TARGET_COMMIT,
    "graph-replay": GRAPH_ADAPTED_TARGET_COMMIT,
    "structural-delta-cas": GRAPH_ADAPTED_TARGET_COMMIT,
    "fxp6-arithmetic": DYNAMICS_TARGET_COMMIT,
    "jacobi-before-state": DYNAMICS_TARGET_COMMIT,
    "eight-dof-state": DYNAMICS_TARGET_COMMIT,
    "energy-telemetry": DYNAMICS_TARGET_COMMIT,
    "capacity-telemetry": DYNAMICS_TARGET_COMMIT,
    "renormalization-residual-telemetry": DYNAMICS_TARGET_COMMIT,
    "semantic-vector-receipt-v2": SEMANTIC_MIGRATION_TARGET_COMMIT,
    "node-observability-v2": SEMANTIC_MIGRATION_TARGET_COMMIT,
    "expression-projection-v1": SEMANTIC_MIGRATION_TARGET_COMMIT,
    "aesem2-read-compatibility": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    "aesem3-current-write": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    "finite-domain-migration": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    "migration-preimage-backup": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    "formula-upgrade-proof": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    "paired-atomic-commit": PAIRED_SEMANTIC_TARGET_COMMIT,
    "event-deduplication": PAIRED_SEMANTIC_TARGET_COMMIT,
    "stale-base-rejection": PAIRED_SEMANTIC_TARGET_COMMIT,
    "identity-conflict-rejection": PAIRED_SEMANTIC_TARGET_COMMIT,
    "evidence-canonical-codec": TASK7_TARGET_COMMIT,
    "independent-semantic-cursor": TASK7_TARGET_COMMIT,
    "relation-private-evidence": TASK7_TARGET_COMMIT,
    "persona-global-mood": TASK7_TARGET_COMMIT,
}
TASK4_RECEIPT_ARTIFACT_PATHS = {
    "Cargo.lock",
    "crates/ae-runtime/Cargo.toml",
    "crates/ae-runtime/src/lib.rs",
    "crates/ae-runtime/src/semantic_dynamics_v2.rs",
    "crates/ae-runtime/src/semantic_telemetry_v1.rs",
    "crates/ae-runtime/tests/emotion_matrix_dynamics.rs",
    "crates/ae-contracts/src/emotion_matrix.rs",
    "crates/ae-neurofield/src/lib.rs",
    "crates/ae-attention/src/emotion_matrix.rs",
}
TASK5_RECEIPT_ARTIFACT_PATHS = {
    "Cargo.lock",
    "crates/ae-runtime/src/lib.rs",
    "crates/ae-runtime/src/semantic.rs",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
    "crates/ae-store/tests/emotion_matrix_migration.rs",
}
TASK5_V2_RUNTIME_ARTIFACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "crates/ae-runtime/Cargo.toml",
    "crates/ae-runtime/src/lib.rs",
    "crates/ae-runtime/src/semantic.rs",
}
TASK5_V2_STORE_ARTIFACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
    "crates/ae-store/tests/emotion_matrix_migration_v2.rs",
    "crates/ae-platform-fs/Cargo.toml",
    "crates/ae-platform-fs/src/lib.rs",
}
TASK5_V2_PLATFORM_ARTIFACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "crates/ae-platform-fs/Cargo.toml",
    "crates/ae-platform-fs/src/lib.rs",
}
TASK6_PRODUCTION_ARTIFACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic.rs",
    "crates/ae-store/src/semantic_atomic_tests.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
}
TASK6_FIXTURE_ARTIFACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
    "crates/ae-store/tests/emotion_matrix_migration_v2.rs",
    "crates/ae-platform-fs/Cargo.toml",
    "crates/ae-platform-fs/src/lib.rs",
}
TASK7_CORE_ARTIFACT_PATHS = {
    "Cargo.lock", "Cargo.toml",
    "crates/ae-contracts/Cargo.toml",
    "crates/ae-contracts/src/emotion_matrix.rs",
    "crates/ae-attention/Cargo.toml",
    "crates/ae-attention/src/emotion_matrix.rs",
    "crates/ae-neurofield/Cargo.toml",
    "crates/ae-neurofield/src/lib.rs",
    "crates/ae-semantic-core/Cargo.toml",
    "crates/ae-semantic-core/src/lib.rs",
    "crates/ae-semantic-core/src/codec.rs",
    "crates/ae-semantic-core/src/semantic_dynamics_v2.rs",
    "crates/ae-semantic-core/src/semantic_telemetry_v1.rs",
    "crates/ae-semantic-core/tests/user_stimulus_transition.rs",
}
TASK7_RUNTIME_COMMON_ARTIFACT_PATHS = {
    "Cargo.lock", "Cargo.toml",
    "crates/ae-contracts/src/emotion_matrix.rs",
    "crates/ae-semantic-core/Cargo.toml",
    "crates/ae-semantic-core/src/lib.rs",
    "crates/ae-semantic-core/src/codec.rs",
    "crates/ae-semantic-core/src/semantic_dynamics_v2.rs",
    "crates/ae-semantic-core/src/semantic_telemetry_v1.rs",
    "crates/ae-runtime/Cargo.toml",
    "crates/ae-runtime/src/lib.rs",
    "crates/ae-runtime/src/semantic.rs",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
}
TASK7_EVENT_ARTIFACT_PATHS = TASK7_RUNTIME_COMMON_ARTIFACT_PATHS | {
    "crates/ae-runtime/tests/emotion_matrix_event_lane.rs",
}
TASK7_HOST_ARTIFACT_PATHS = TASK7_RUNTIME_COMMON_ARTIFACT_PATHS | {
    "crates/ae-runtime/tests/perception_origin_authority.rs",
}
TASK7_STORE_ARTIFACT_PATHS = {
    "Cargo.lock", "Cargo.toml",
    "crates/ae-contracts/src/emotion_matrix.rs",
    "crates/ae-semantic-core/Cargo.toml",
    "crates/ae-semantic-core/src/lib.rs",
    "crates/ae-semantic-core/src/codec.rs",
    "crates/ae-semantic-core/src/semantic_dynamics_v2.rs",
    "crates/ae-semantic-core/src/semantic_telemetry_v1.rs",
    "crates/ae-store/Cargo.toml",
    "crates/ae-store/src/lib.rs",
    "crates/ae-store/src/semantic.rs",
    "crates/ae-store/src/semantic_atomic_tests.rs",
    "crates/ae-store/src/semantic_field_attestation.rs",
}
EXPECTED_RECEIPT_ARTIFACT_PATHS = {
    "emotion-matrix-task2-contracts-exact": {
        "Cargo.lock",
        "crates/ae-contracts/src/emotion_matrix.rs",
        "crates/ae-contracts/src/lib.rs",
        "crates/ae-contracts/tests/emotion_matrix_contract.rs",
    },
    "emotion-matrix-task2-attention-exact": {
        "Cargo.lock",
        "crates/ae-contracts/src/emotion_matrix.rs",
        "crates/ae-attention/Cargo.toml",
        "crates/ae-attention/src/emotion_matrix.rs",
        "crates/ae-attention/tests/emotion_matrix_route.rs",
    },
    "emotion-matrix-task3-graph-development-exact": {
        "Cargo.lock",
        "crates/ae-neurofield/Cargo.toml",
        "crates/ae-neurofield/src/lib.rs",
        "crates/ae-neurofield/src/graph_development.rs",
        "crates/ae-neurofield/tests/deterministic_graph_development.rs",
        "crates/ae-neurofield/tests/vectors/graph-development-v1.json",
    },
    "emotion-matrix-task3-graph-replay-exact": {
        "Cargo.lock",
        "crates/ae-neurofield/Cargo.toml",
        "crates/ae-neurofield/src/lib.rs",
        "crates/ae-neurofield/src/graph_development.rs",
        "crates/ae-neurofield/src/graph_replay.rs",
        "crates/ae-neurofield/src/structural_delta.rs",
        "crates/ae-neurofield/tests/graph_admission_equivalence.rs",
        "crates/ae-neurofield/tests/vectors/graph-replay-source-v1-nonempty.json",
        "crates/ae-neurofield/tests/vectors/structural-delta-source-v1-composite.bin",
        "crates/ae-neurofield/tests/vectors/task3-adapted-source-v1-fixtures.provenance.json",
    },
    "emotion-matrix-task3-structural-delta-exact": {
        "Cargo.lock",
        "crates/ae-neurofield/Cargo.toml",
        "crates/ae-neurofield/src/lib.rs",
        "crates/ae-neurofield/src/graph_development.rs",
        "crates/ae-neurofield/src/graph_replay.rs",
        "crates/ae-neurofield/src/structural_delta.rs",
        "crates/ae-neurofield/tests/graph_admission_equivalence.rs",
        "crates/ae-neurofield/tests/vectors/graph-replay-source-v1-nonempty.json",
        "crates/ae-neurofield/tests/vectors/structural-delta-source-v1-composite.bin",
        "crates/ae-neurofield/tests/vectors/task3-adapted-source-v1-fixtures.provenance.json",
    },
    "emotion-matrix-task4-dynamics-exact": TASK4_RECEIPT_ARTIFACT_PATHS,
    "emotion-matrix-task4-telemetry-exact": TASK4_RECEIPT_ARTIFACT_PATHS,
    "emotion-matrix-task4-error-atomicity-exact": TASK4_RECEIPT_ARTIFACT_PATHS,
    "emotion-matrix-task4-runtime-check": TASK4_RECEIPT_ARTIFACT_PATHS,
    "emotion-matrix-task5-runtime-semantic-exact": TASK5_RECEIPT_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-runtime-aesem2-bounded": TASK5_V2_RUNTIME_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-runtime-aesem3-four-block": TASK5_V2_RUNTIME_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-finite-copy-verify-commit": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-authority-closure": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-formula-authority": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-sql-type-gates": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-sql-size-gate": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-creator-type-gate": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-backup-18-seam": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-rowless-cross-version-recovery": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-anchored-package": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-platform-anchored-package": TASK5_V2_PLATFORM_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-cas-atomicity": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-sql-rollback": TASK5_V2_STORE_ARTIFACT_PATHS,
    "emotion-matrix-task5-v2-store-reopen-reattest": TASK5_V2_STORE_ARTIFACT_PATHS,
}
EXPECTED_RECEIPT_TARGET_DIRS = {
    "emotion-matrix-task2-contracts-exact": TASK2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task2-attention-exact": TASK2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task3-graph-development-exact": TASK3_RECEIPT_TARGET_DIR,
    "emotion-matrix-task3-graph-replay-exact": TASK3_RECEIPT_TARGET_DIR,
    "emotion-matrix-task3-structural-delta-exact": TASK3_RECEIPT_TARGET_DIR,
    "emotion-matrix-task4-dynamics-exact": TASK4_RECEIPT_TARGET_DIR,
    "emotion-matrix-task4-telemetry-exact": TASK4_RECEIPT_TARGET_DIR,
    "emotion-matrix-task4-error-atomicity-exact": TASK4_RECEIPT_TARGET_DIR,
    "emotion-matrix-task4-runtime-check": TASK4_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-runtime-semantic-exact": TASK5_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-runtime-aesem2-bounded": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-runtime-aesem3-four-block": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-finite-copy-verify-commit": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-authority-closure": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-formula-authority": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-sql-type-gates": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-sql-size-gate": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-creator-type-gate": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-backup-18-seam": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-rowless-cross-version-recovery": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-anchored-package": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-platform-anchored-package": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-cas-atomicity": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-sql-rollback": TASK5_V2_RECEIPT_TARGET_DIR,
    "emotion-matrix-task5-v2-store-reopen-reattest": TASK5_V2_RECEIPT_TARGET_DIR,
}

TASK6_PRODUCTION_RECEIPT_NAMES = {
    "store-authority-boundary-default",
    "store-atomicity",
    "store-dedupe-stale-conflict",
    "store-missing-cursor-origin",
    "store-journal-only-rejection",
    "store-authority-rebinding",
    "store-relation-identity-closure",
    "store-history-closure",
    "store-historical-payload-closure",
    "store-weak-schema-migration",
    "store-authority-boundary-all-features",
    "store-check-default",
    "store-check-all-features",
}
TASK6_FIXTURE_RECEIPT_NAMES = {
    "task5-finite-migration-compat",
    "task5-authority-closure-compat",
    "task5-backup-seams-compat",
    "task5-formula-authority-compat",
    "task5-reopen-reattest-compat",
}
for receipt_name in TASK6_PRODUCTION_RECEIPT_NAMES:
    receipt_id = f"emotion-matrix-task6-v1-{receipt_name}"
    EXPECTED_RECEIPT_ARTIFACT_PATHS[receipt_id] = TASK6_PRODUCTION_ARTIFACT_PATHS
    EXPECTED_RECEIPT_TARGET_DIRS[receipt_id] = TASK6_RECEIPT_TARGET_DIR
for receipt_name in TASK6_FIXTURE_RECEIPT_NAMES:
    receipt_id = f"emotion-matrix-task6-v1-{receipt_name}"
    EXPECTED_RECEIPT_ARTIFACT_PATHS[receipt_id] = TASK6_FIXTURE_ARTIFACT_PATHS
    EXPECTED_RECEIPT_TARGET_DIRS[receipt_id] = TASK6_RECEIPT_TARGET_DIR
for receipt_name, artifact_paths in {
    "semantic-core-transition": TASK7_CORE_ARTIFACT_PATHS,
    "runtime-evolution-reopen": TASK7_EVENT_ARTIFACT_PATHS,
    "runtime-relation-private-persona-global": TASK7_EVENT_ARTIFACT_PATHS,
    "runtime-invalid-stale-atomic": TASK7_EVENT_ARTIFACT_PATHS,
    "store-dedupe-stale-identity": TASK7_STORE_ARTIFACT_PATHS,
    "host-origin-authority-retry": TASK7_HOST_ARTIFACT_PATHS,
    "host-nonhost-rejection": TASK7_HOST_ARTIFACT_PATHS,
}.items():
    receipt_id = f"emotion-matrix-task7-v1-{receipt_name}"
    EXPECTED_RECEIPT_ARTIFACT_PATHS[receipt_id] = artifact_paths
    EXPECTED_RECEIPT_TARGET_DIRS[receipt_id] = TASK7_RECEIPT_TARGET_DIR


def _git(*args: str, cwd: Path = ROOT, binary: bool = False):
    result = subprocess.run(
        ["git", "-C", str(cwd), *args], check=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    return result.stdout if binary else result.stdout.decode("utf-8").strip()


def _source_blob(path: str) -> bytes:
    return _git("cat-file", "blob", f"{SOURCE_COMMIT}:{path}", cwd=SOURCE_ROOT, binary=True)


def _commit_exists(commit: str) -> bool:
    if not isinstance(commit, str) or FULL_OBJECT_ID.fullmatch(commit) is None:
        return False
    result = subprocess.run(
        ["git", "-C", str(ROOT), "cat-file", "-e", f"{commit}^{{commit}}"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    return result.returncode == 0


def _resolved_evidence_file(locator: str) -> Path:
    assert isinstance(locator, str) and any(
        locator.startswith(root) for root in RECEIPT_EVIDENCE_ROOTS
    )
    relative = Path(locator)
    assert not relative.is_absolute() and relative.as_posix() == locator
    resolved = (ROOT / relative).resolve()
    assert resolved.is_relative_to(ROOT.resolve()) and resolved.is_file()
    return resolved


def _validate_execution_receipt(
    evidence: dict,
    expected_command: str,
    target_commits: set[str],
    *,
    exact_test: bool = True,
    expected_list_count: int | None = None,
) -> None:
    assert set(evidence) == EVIDENCE_KEYS
    assert evidence["kind"] == "local_reproducible_execution_receipt"
    assert SHA256.fullmatch(evidence["sha256"])
    assert evidence["target_commit"] in target_commits
    receipt_path = _resolved_evidence_file(evidence["locator"])
    receipt_bytes = receipt_path.read_bytes()
    assert hashlib.sha256(receipt_bytes).hexdigest() == evidence["sha256"]
    assert _git("show", f"HEAD:{evidence['locator']}", binary=True) == receipt_bytes
    receipt = json.loads(receipt_bytes.decode("utf-8"))
    assert set(receipt) == {
        "schema", "evidence_class", "authority_claim", "receipt_id", "target_commit",
        "command", "argv", "cwd", "execution", "toolchain", "logs", "artifacts",
    }
    assert receipt["schema"] == RECEIPT_SCHEMA
    assert receipt["evidence_class"] == "LOCAL_REPRODUCIBLE"
    assert receipt["authority_claim"] == "LOCAL_REPRODUCIBLE_EVIDENCE_ONLY"
    assert receipt["target_commit"] == evidence["target_commit"]
    assert receipt["command"] == expected_command
    assert receipt["command"] == "cargo " + " ".join(receipt["argv"])
    assert "--locked" in receipt["argv"] and "--offline" in receipt["argv"]
    if expected_list_count is not None:
        assert receipt["argv"][-2:] == ["--", "--list"]
    elif exact_test:
        assert receipt["argv"][-2:] == ["--", "--exact"] or receipt["argv"][-3:] == [
            "--", "--exact", "--nocapture"
        ]
    else:
        assert receipt["argv"][0] == "check"
    assert receipt["cwd"] == RECEIPT_CWD
    expected_target_dir = EXPECTED_RECEIPT_TARGET_DIRS[receipt["receipt_id"]]
    assert receipt["execution"] == {
        "exit_code": 0,
        "locked": True,
        "offline": True,
        "cargo_target_dir": expected_target_dir,
        "cargo_term_color": "never",
    }
    assert receipt["toolchain"] == EXPECTED_TOOLCHAIN

    logs = receipt["logs"]
    assert set(logs) == {"normalization", "stdout", "stderr"}
    assert logs["normalization"] == "utf8-lf"
    resolved_logs = {}
    for stream in ("stdout", "stderr"):
        descriptor = logs[stream]
        assert set(descriptor) == {"path", "sha256"}
        assert SHA256.fullmatch(descriptor["sha256"])
        log_path = _resolved_evidence_file(descriptor["path"])
        log_bytes = log_path.read_bytes()
        assert b"\r" not in log_bytes
        assert hashlib.sha256(log_bytes).hexdigest() == descriptor["sha256"]
        assert _git("show", f"HEAD:{descriptor['path']}", binary=True) == log_bytes
        resolved_logs[stream] = log_bytes.decode("utf-8")
    if expected_list_count is not None:
        assert f"{expected_list_count} tests, 0 benchmarks" in resolved_logs["stdout"]
        assert resolved_logs["stdout"].count(": test\n") == expected_list_count
        assert "Finished `test` profile" in resolved_logs["stderr"]
        assert expected_target_dir in resolved_logs["stderr"]
    elif exact_test:
        assert "test result: ok. 1 passed; 0 failed;" in resolved_logs["stdout"]
        assert "Finished `test` profile" in resolved_logs["stderr"]
        assert expected_target_dir in resolved_logs["stderr"]
        if receipt["receipt_id"] in {
            "emotion-matrix-task5-v2-store-backup-18-seam",
            "emotion-matrix-task6-v1-task5-backup-seams-compat",
        }:
            expected_seams = {
                "before-snapshot", "after-snapshot", "before-database-sync",
                "after-database-sync", "before-manifest-sync", "after-manifest-sync",
                "before-directory-sync", "after-directory-sync", "before-atomic-publish",
                "after-atomic-publish", "before-parent-sync", "after-parent-sync",
                "before-transaction-begin", "after-transaction-begin",
                "before-row-insertion", "after-row-insertion", "before-commit", "after-commit",
            }
            seam_occurrences = re.findall(
                r"semantic-migration-failpoint=([a-z-]+)",
                resolved_logs["stdout"],
            )
            seams = set(seam_occurrences)
            assert seams == expected_seams
            assert len(seam_occurrences) == 18
    else:
        assert resolved_logs["stdout"] == ""
        assert "Finished `dev` profile" in resolved_logs["stderr"]

    artifacts = receipt["artifacts"]
    assert artifacts and all(set(artifact) == {"path", "sha256"} for artifact in artifacts)
    artifact_paths = [artifact["path"] for artifact in artifacts]
    assert len(artifact_paths) == len(set(artifact_paths))
    assert set(artifact_paths) == EXPECTED_RECEIPT_ARTIFACT_PATHS[receipt["receipt_id"]]
    for artifact in artifacts:
        assert SHA256.fullmatch(artifact["sha256"])
        raw = _git(
            "show",
            f"{receipt['target_commit']}:{artifact['path']}",
            binary=True,
        )
        assert hashlib.sha256(raw).hexdigest() == artifact["sha256"]


def _ref_key(ref: dict) -> tuple[str, str]:
    return ref["path"], ref["qualified_name"]


def _assert_source_definition(ref: dict, text: str) -> None:
    qualified = ref["qualified_name"]
    parts = re.split(r"::|\.", qualified)
    for name in ({parts[-1], parts[-2]} if len(parts) > 1 else {parts[-1]}):
        assert re.search(rf"(?<![A-Za-z0-9_]){re.escape(name)}(?![A-Za-z0-9_])", text), ref


def _validate_provenance(provenance: dict) -> dict[tuple[str, str], dict]:
    assert provenance["schema"] == "ae.emotion-matrix-provenance.v1"
    assert provenance["hash_profile"] == HASH_PROFILE
    assert provenance["source_commit"] == SOURCE_COMMIT
    assert provenance["target_baseline"] == TARGET_BASELINE
    assert provenance["merge_base"] == MERGE_BASE
    assert provenance["audited_source_commits"] == EXPECTED_AUDITED_SOURCE_COMMITS
    files = provenance["files"]
    assert len(files) == 20
    assert len({row["source_path"] for row in files}) == 20
    assert {row["source_path"]: row["sha256"] for row in files} == EXPECTED_SOURCE_BLOBS
    universe: dict[tuple[str, str], dict] = {}
    for row in files:
        assert set(row) == {"source_commit", "source_path", "source_blob", "sha256", "source_symbols", "target_symbols"}
        path = row["source_path"]
        raw = _source_blob(path)
        blob = _git("rev-parse", f"{SOURCE_COMMIT}:{path}", cwd=SOURCE_ROOT)
        assert row["source_commit"] == SOURCE_COMMIT
        assert row["source_blob"] == blob
        assert hashlib.sha256(raw).hexdigest() == row["sha256"]
        text = raw.decode("utf-8")
        assert row["source_symbols"] and row["target_symbols"]
        assert len(row["source_symbols"]) == len(row["target_symbols"])
        for source_ref, target_ref in zip(row["source_symbols"], row["target_symbols"]):
            assert set(source_ref) == SOURCE_REF_KEYS
            assert source_ref["path"] == path
            assert source_ref["commit"] == SOURCE_COMMIT
            assert source_ref["blob"] == blob
            assert source_ref["kind"] in {"rust_item", "python_item", "toml_entry"}
            assert source_ref["qualified_name"]
            _assert_source_definition(source_ref, text)
            key = _ref_key(source_ref)
            assert key not in universe
            universe[key] = source_ref
            assert set(target_ref) == TARGET_REF_KEYS
            assert target_ref["path"] and target_ref["qualified_name"]
            assert target_ref["kind"] in {"rust_item", "python_item", "toml_entry", "artifact"}
            assert target_ref["commit"] is None
    return universe


def _validate_parity(parity: dict, universe: dict[tuple[str, str], dict]) -> dict:
    assert parity["schema"] == "ae.emotion-matrix-capability-parity.v1"
    assert parity["source_commit"] == SOURCE_COMMIT
    assert parity["target_baseline"] == TARGET_BASELINE
    assert parity["merge_base"] == MERGE_BASE
    capabilities = parity["capabilities"]
    ids = [row["id"] for row in capabilities]
    assert len(ids) == len(set(ids))
    assert REQUIRED_CAPABILITY_IDS <= set(ids)
    referenced: set[tuple[str, str]] = set()
    unmapped = 0
    for row in capabilities:
        assert set(row) == {"id", "source_refs", "target_refs", "disposition", "future_test_node", "evidence"}
        disposition = row["disposition"]
        assert disposition in ALLOWED_DISPOSITIONS
        assert row["source_refs"] and row["target_refs"] and row["future_test_node"]
        row_ref_keys = [_ref_key(ref) for ref in row["source_refs"]]
        assert len(row_ref_keys) == len(set(row_ref_keys))
        for ref in row["source_refs"]:
            assert set(ref) == SOURCE_REF_KEYS
            key = _ref_key(ref)
            assert key in universe and ref == universe[key]
            referenced.add(key)
        for ref in row["target_refs"]:
            assert set(ref) == TARGET_REF_KEYS
            assert ref["path"] and ref["kind"] and ref["qualified_name"]
        if disposition == "UNMAPPED":
            unmapped += 1
            assert row["evidence"] == []
            assert all(ref["commit"] is None for ref in row["target_refs"])
            continue
        target_commits = {ref["commit"] for ref in row["target_refs"]}
        assert None not in target_commits
        assert all(_commit_exists(commit) for commit in target_commits)
        for ref in row["target_refs"]:
            try:
                target = _git("show", f"{ref['commit']}:{ref['path']}", binary=True)
            except subprocess.CalledProcessError as error:
                raise AssertionError(f"missing pinned target: {ref}") from error
            _assert_source_definition(ref, target.decode("utf-8"))
        assert len(row["evidence"]) == 1
        _validate_execution_receipt(row["evidence"][0], row["future_test_node"], target_commits)
    assert set(referenced) == set(universe)
    computed_gate = {
        "status": "NO_GO" if unmapped else "GO",
        "reasons": ["UNMAPPED_CAPABILITY"] if unmapped else [],
        "validated_capability_count": len(capabilities),
        "unmapped_capability_count": unmapped,
    }
    assert parity["release_gate"] == computed_gate
    return computed_gate


def _load_and_validate() -> tuple[dict, dict, dict]:
    provenance = json.loads((ROOT / "model/emotion-matrix-provenance-v1.json").read_text("utf-8"))
    parity = json.loads((ROOT / "model/emotion-matrix-capability-parity-v1.json").read_text("utf-8"))
    universe = _validate_provenance(provenance)
    return provenance, parity, _validate_parity(parity, universe)


def test_task5_v1_stays_superseded_and_v2_evidence_is_earned():
    parity = json.loads(
        (ROOT / "model/emotion-matrix-capability-parity-v1.json").read_text("utf-8")
    )
    rows = {row["id"]: row for row in parity["capabilities"]}
    assert TASK5_REINSTATED_CAPABILITY_IDS <= set(rows)
    for capability_id in TASK5_REINSTATED_CAPABILITY_IDS:
        row = rows[capability_id]
        assert row["disposition"] == "ADAPTED_WITH_EQUIVALENCE"
        assert row["evidence"] == [EXPECTED_CAPABILITY_RECEIPTS[capability_id]]
        assert {ref["commit"] for ref in row["target_refs"]} == {
            ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT
        }
    supersession = json.loads(
        (
            ROOT
            / "model/evidence/emotion-matrix-task5-v1/evidence-supersession-v1.json"
        ).read_text("utf-8")
    )
    assert supersession["status"] == "SUPERSEDED_UNEARNED"
    assert supersession["superseded_evidence_commit"] == SUPERSEDED_TASK5_EVIDENCE_COMMIT
    assert set(supersession["affected_capability_ids"]) == (
        TASK5_REINSTATED_CAPABILITY_IDS
    )
    assert supersession["parity_after_rollback"] == {
        "mapped_capability_count": 16,
        "unmapped_capability_count": 27,
        "release_gate": "NO_GO",
    }

    reinstatement_path = (
        ROOT
        / "model/evidence/emotion-matrix-task5-v2/evidence-reinstatement-v2.json"
    )
    reinstatement = json.loads(reinstatement_path.read_text("utf-8"))
    assert set(reinstatement) == {
        "schema", "status", "code_anchor", "review_preconditions",
        "historical_supersession", "affected_capability_ids", "primary_receipts",
        "supporting_receipts", "non_vacuity", "parity_after_reinstatement", "custody_note",
    }
    assert reinstatement["schema"] == "ae.evidence-reinstatement.v2"
    assert reinstatement["status"] == "EARNED_AFTER_REMEDIATION"
    assert reinstatement["code_anchor"] == ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT
    assert reinstatement["review_preconditions"] == {
        "code_spec": "APPROVED",
        "quality": "APPROVED",
        "reviewed_commit": ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT,
    }
    assert reinstatement["historical_supersession"] == {
        "locator": "model/evidence/emotion-matrix-task5-v1/evidence-supersession-v1.json",
        "status": "SUPERSEDED_UNEARNED",
        "superseded_evidence_commit": SUPERSEDED_TASK5_EVIDENCE_COMMIT,
        "remains_superseded": True,
    }
    assert set(reinstatement["affected_capability_ids"]) == TASK5_REINSTATED_CAPABILITY_IDS
    expected_primary = {
        capability_id: {
            "locator": receipt["locator"],
            "sha256": receipt["sha256"],
        }
        for capability_id, receipt in EXPECTED_CAPABILITY_RECEIPTS.items()
        if capability_id in TASK5_REINSTATED_CAPABILITY_IDS
    }
    assert reinstatement["primary_receipts"] == expected_primary
    support_by_locator = {
        row["locator"]: row for row in reinstatement["supporting_receipts"]
    }
    expected_support = {
        receipt["locator"]: receipt
        for receipt in (
            TASK5_AESEM3_RECEIPT,
            TASK5_SQL_TYPE_RECEIPT,
            TASK5_SQL_SIZE_RECEIPT,
            TASK5_CREATOR_TYPE_RECEIPT,
            TASK5_ROWLESS_RECEIPT,
            TASK5_STORE_PACKAGE_RECEIPT,
            TASK5_PLATFORM_PACKAGE_RECEIPT,
            TASK5_CAS_RECEIPT,
            TASK5_SQL_ROLLBACK_RECEIPT,
            TASK5_REOPEN_RECEIPT,
        )
    }
    assert set(support_by_locator) == set(expected_support)
    for locator, receipt in expected_support.items():
        assert support_by_locator[locator]["sha256"] == receipt["sha256"]
        assert support_by_locator[locator]["exact_test_count"] == 1
    assert reinstatement["non_vacuity"] == {
        "exact_receipt_count": 15,
        "passed_exact_test_count": 15,
        "backup_failpoint_seam_count": 18,
    }
    assert reinstatement["parity_after_reinstatement"] == {
        "mapped_capability_count": 21,
        "unmapped_capability_count": 22,
        "release_gate": "NO_GO",
    }
    active_evidence = [
        evidence
        for row in parity["capabilities"]
        for evidence in row["evidence"]
    ]
    assert all(evidence["target_commit"] != SUPERSEDED_TASK5_EVIDENCE_COMMIT for evidence in active_evidence)
    assert all("emotion-matrix-task5-v1/runtime-aesem2" not in evidence["locator"] for evidence in active_evidence)
    assert parity["release_gate"] == {
        "status": "NO_GO",
        "reasons": ["UNMAPPED_CAPABILITY"],
        "validated_capability_count": 43,
        "unmapped_capability_count": 14,
    }


def test_task6_evidence_custody_pins_production_and_fixture_anchors_separately():
    custody_path = ROOT / f"{TASK6_RECEIPT_EVIDENCE_ROOT}evidence-custody-v1.json"
    custody = json.loads(custody_path.read_text("utf-8"))
    assert set(custody) == {
        "schema", "status", "production_code_anchor", "fixture_compatibility_anchor",
        "review_preconditions", "affected_capability_ids", "held_for_later_task_ids",
        "primary_receipts", "supporting_exact_receipts", "compile_receipts",
        "non_vacuity", "parity_after_task6", "custody_note",
    }
    assert custody["schema"] == "ae.emotion-matrix-task6-evidence-custody.v1"
    assert custody["status"] == "EARNED_AFTER_REVIEW"
    assert custody["production_code_anchor"] == PAIRED_SEMANTIC_TARGET_COMMIT
    assert custody["fixture_compatibility_anchor"] == TASK6_FIXTURE_TARGET_COMMIT
    assert custody["review_preconditions"] == {
        "quality": "APPROVED",
        "quality_reviewed_commit": PAIRED_SEMANTIC_TARGET_COMMIT,
        "code_spec": "APPROVED",
        "code_spec_reviewed_commit": TASK6_FIXTURE_TARGET_COMMIT,
        "fixture_delta_changes_production": False,
    }
    task6_capabilities = {
        "paired-atomic-commit", "event-deduplication",
        "stale-base-rejection", "identity-conflict-rejection",
    }
    assert set(custody["affected_capability_ids"]) == task6_capabilities
    assert custody["held_for_later_task_ids"] == ["independent-semantic-cursor"]
    assert _git(
        "diff", "--name-only",
        f"{PAIRED_SEMANTIC_TARGET_COMMIT}..{TASK6_FIXTURE_TARGET_COMMIT}",
    ) == "crates/ae-store/tests/emotion_matrix_migration_v2.rs"

    expected_primary = {
        capability_id: {
            "locator": receipt["locator"],
            "sha256": receipt["sha256"],
            "target_commit": receipt["target_commit"],
        }
        for capability_id, receipt in EXPECTED_CAPABILITY_RECEIPTS.items()
        if capability_id in task6_capabilities
    }
    assert custody["primary_receipts"] == expected_primary

    exact_rows = list(custody["supporting_exact_receipts"])
    primary_unique = {
        (row["locator"], row["sha256"], row["target_commit"])
        for row in custody["primary_receipts"].values()
    }
    for locator, digest, target_commit in primary_unique:
        exact_rows.append({
            "claim": "primary",
            "locator": locator,
            "sha256": digest,
            "target_commit": target_commit,
        })
    assert len(exact_rows) == 16
    assert len({row["locator"] for row in exact_rows}) == 16
    expected_exact_names = (
        TASK6_PRODUCTION_RECEIPT_NAMES - {"store-check-default", "store-check-all-features"}
    ) | TASK6_FIXTURE_RECEIPT_NAMES
    assert {
        Path(row["locator"]).name.removesuffix(".receipt.json") for row in exact_rows
    } == expected_exact_names
    for row in exact_rows:
        evidence = {
            "kind": "local_reproducible_execution_receipt",
            "locator": row["locator"],
            "sha256": row["sha256"],
            "target_commit": row["target_commit"],
        }
        receipt = json.loads(_resolved_evidence_file(row["locator"]).read_text("utf-8"))
        _validate_execution_receipt(evidence, receipt["command"], {row["target_commit"]})

    assert {row["configuration"] for row in custody["compile_receipts"]} == {
        "default", "all-features",
    }
    for row in custody["compile_receipts"]:
        evidence = {
            "kind": "local_reproducible_execution_receipt",
            "locator": row["locator"],
            "sha256": row["sha256"],
            "target_commit": row["target_commit"],
        }
        receipt = json.loads(_resolved_evidence_file(row["locator"]).read_text("utf-8"))
        _validate_execution_receipt(
            evidence, receipt["command"], {row["target_commit"]}, exact_test=False,
        )

    assert custody["non_vacuity"] == {
        "private_semantic_test_identity_count": 10,
        "exact_receipt_count": 16,
        "passed_exact_test_count": 16,
        "task5_fixture_compatibility_exact_count": 5,
        "compile_receipt_count": 2,
        "backup_failpoint_seam_count": 18,
    }
    assert custody["parity_after_task6"] == {
        "mapped_capability_count": 25,
        "unmapped_capability_count": 18,
        "release_gate": "NO_GO",
    }


def test_task6_semantic_commit_authority_stays_private_in_all_feature_sets():
    store_lib = (ROOT / "crates/ae-store/src/lib.rs").read_text("utf-8")
    semantic = (ROOT / "crates/ae-store/src/semantic.rs").read_text("utf-8")
    cargo = (ROOT / "crates/ae-store/Cargo.toml").read_text("utf-8")
    assert "\nmod semantic;" in store_lib
    assert "pub mod semantic;" not in store_lib
    assert "pub(crate) struct PairedSemanticCommitV1" in semantic
    assert "pub(crate) fn commit_event_with_semantic_v1(" in semantic
    assert "#[cfg(test)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub(crate) enum SemanticFaultPoint" in semantic
    assert "semantic-test-hooks" not in cargo


def test_task7_trusted_semantic_evolution_evidence_is_pinned_and_non_vacuous():
    custody = json.loads(
        (ROOT / f"{TASK7_RECEIPT_EVIDENCE_ROOT}evidence-custody-v1.json").read_text("utf-8")
    )
    assert set(custody) == {
        "schema", "status", "code_anchor", "review_preconditions", "prior_parity",
        "newly_mapped_capability_ids", "primary_receipts",
        "supporting_claim_receipts", "exact_receipts", "semantic_core_extraction",
        "host_trust_boundary", "non_vacuity", "parity_after_task7", "custody_note",
    }
    assert custody["schema"] == "ae.emotion-matrix-task7-evidence-custody.v1"
    assert custody["status"] == "EARNED_AFTER_REVIEW"
    assert custody["code_anchor"] == TASK7_TARGET_COMMIT
    assert custody["review_preconditions"] == {
        "code_spec": "APPROVED",
        "quality": "APPROVED",
        "reviewed_commit": TASK7_TARGET_COMMIT,
    }
    newly_mapped = {
        "evidence-canonical-codec", "independent-semantic-cursor",
        "relation-private-evidence", "persona-global-mood",
    }
    assert set(custody["newly_mapped_capability_ids"]) == newly_mapped
    assert custody["prior_parity"] == {
        "mapped_capability_count": 25,
        "unmapped_capability_count": 18,
        "release_gate": "NO_GO",
    }
    expected_primary = {
        capability_id: {
            "locator": EXPECTED_CAPABILITY_RECEIPTS[capability_id]["locator"],
            "sha256": EXPECTED_CAPABILITY_RECEIPTS[capability_id]["sha256"],
            "target_commit": TASK7_TARGET_COMMIT,
        }
        for capability_id in newly_mapped
    }
    assert custody["primary_receipts"] == expected_primary

    command_by_locator = {
        TASK7_CORE_RECEIPT["locator"]: TASK7_CORE_COMMAND,
        TASK7_EVOLUTION_RECEIPT["locator"]: TASK7_EVOLUTION_COMMAND,
        TASK7_RELATION_RECEIPT["locator"]: TASK7_RELATION_COMMAND,
        TASK7_INVALID_RECEIPT["locator"]: TASK7_INVALID_COMMAND,
        TASK7_STORE_RECEIPT["locator"]: TASK7_STORE_COMMAND,
        TASK7_HOST_RECEIPT["locator"]: TASK7_HOST_COMMAND,
        TASK7_NONHOST_RECEIPT["locator"]: TASK7_NONHOST_COMMAND,
    }
    exact_rows = custody["exact_receipts"]
    assert len(exact_rows) == 7
    assert len({row["locator"] for row in exact_rows}) == 7
    for row in exact_rows:
        assert set(row) == {"claims", "locator", "sha256", "target_commit"}
        assert row["claims"]
        evidence = {
            "kind": "local_reproducible_execution_receipt",
            "locator": row["locator"],
            "sha256": row["sha256"],
            "target_commit": row["target_commit"],
        }
        _validate_execution_receipt(
            evidence,
            command_by_locator[row["locator"]],
            {TASK7_TARGET_COMMIT},
        )

    supporting = custody["supporting_claim_receipts"]
    expected_supporting_claims = {
        "paired-atomic-commit", "event-deduplication", "stale-base-rejection",
        "identity-conflict-rejection", "semantic-vector-receipt-v2",
        "aesem3-current-write", "energy-telemetry", "capacity-telemetry",
        "renormalization-residual-telemetry",
    }
    assert set(supporting) == expected_supporting_claims
    for capability_id, rows in supporting.items():
        assert rows
        for row in rows:
            assert capability_id in row["claims"]
            assert row in exact_rows

    extraction = custody["semantic_core_extraction"]
    assert extraction["extraction_commit"] == TASK7_EXTRACTION_COMMIT
    assert extraction["pre_extraction_parent"] == TASK7_EXTRACTION_PARENT
    exact = extraction["exact_blob_equivalence"]
    assert exact["scope"] == "whole-file semantic_dynamics_v2 only"
    assert {
        exact["source"]["git_blob"],
        exact["extracted"]["git_blob"],
        exact["reviewed_head"]["git_blob"],
    } == {"69d6a27fed7faec98c8a7eddd1a09484dfcd758d"}
    for descriptor in (exact["source"], exact["extracted"], exact["reviewed_head"]):
        assert _git("rev-parse", f"{descriptor['commit']}:{descriptor['path']}") == descriptor["git_blob"]
    for descriptor in extraction["adapted_core_blobs"]:
        assert _git(
            "rev-parse", f"{TASK7_TARGET_COMMIT}:{descriptor['path']}"
        ) == descriptor["git_blob"]
        raw = _git(
            "show", f"{TASK7_TARGET_COMMIT}:{descriptor['path']}", binary=True
        )
        assert hashlib.sha256(raw).hexdigest() == descriptor["sha256"]
    assert extraction["adapted_equivalence_evidence"] == TASK7_CORE_RECEIPT["locator"]

    trust = custody["host_trust_boundary"]
    assert "trusted" in trust["assumption"]
    assert len(trust["native_enforcement"]) == 3
    assert trust["receipts"] == [
        next(row for row in exact_rows if row["locator"] == TASK7_HOST_RECEIPT["locator"]),
        next(row for row in exact_rows if row["locator"] == TASK7_NONHOST_RECEIPT["locator"]),
    ]
    pyo3 = trust["pyo3_boundary"]
    assert pyo3["code_anchor"] == TASK7_TARGET_COMMIT
    assert pyo3["arbitrary_challenge_mint_exported"] is False
    assert pyo3["raw_user_stimulus_export_rejected"] is True
    assert pyo3["authenticated_proposal_entry_exported"] == "apply_perception_proposal_v1"
    pyo3_source = _git("show", f"{TASK7_TARGET_COMMIT}:{pyo3['path']}", binary=True)
    assert _git("rev-parse", f"{TASK7_TARGET_COMMIT}:{pyo3['path']}") == pyo3["git_blob"]
    assert hashlib.sha256(pyo3_source).hexdigest() == pyo3["sha256"]
    pyo3_text = pyo3_source.decode("utf-8")
    assert "mint_perception_challenge_from_committed_inbound_v1" not in pyo3_text
    assert "UNAUTHENTICATED_USER_STIMULUS::raw UserStimulus is rejected" in pyo3_text
    assert "wrap_pyfunction!(apply_perception_proposal_v1, module)" in pyo3_text

    assert custody["non_vacuity"] == {
        "exact_receipt_count": 7,
        "passed_exact_test_count": 7,
        "distinct_exact_test_identity_count": 7,
        "runtime_end_to_end_exact_count": 5,
        "semantic_core_exact_count": 1,
        "store_exact_count": 1,
    }
    assert custody["parity_after_task7"] == {
        "mapped_capability_count": 29,
        "unmapped_capability_count": 14,
        "release_gate": "NO_GO",
    }


def test_emotion_matrix_provenance_and_parity_are_closed():
    provenance, parity, gate = _load_and_validate()
    universe = _validate_provenance(provenance)
    assert {
        path for path, name in universe if name == "decode_semantic_snapshot_v2"
    } == {
        "crates/ae-runtime/src/semantic.rs",
        "crates/ae-store/src/semantic_field_attestation.rs",
    }
    mapped = {
        row["id"]: row
        for row in parity["capabilities"]
        if row["disposition"] != "UNMAPPED"
    }
    assert set(mapped) == EXPECTED_MAPPED_CAPABILITY_IDS
    for capability_id, row in mapped.items():
        assert row["disposition"] == EXPECTED_CAPABILITY_DISPOSITIONS[capability_id]
        assert {ref["commit"] for ref in row["target_refs"]} == {
            EXPECTED_CAPABILITY_TARGET_COMMITS[capability_id]
        }
        assert row["evidence"] == [EXPECTED_CAPABILITY_RECEIPTS[capability_id]]
    task4_capabilities = {
        "fxp6-arithmetic",
        "jacobi-before-state",
        "eight-dof-state",
        "energy-telemetry",
        "capacity-telemetry",
        "renormalization-residual-telemetry",
    }
    assert {mapped[capability_id]["future_test_node"] for capability_id in task4_capabilities} == {
        DYNAMICS_COMMAND,
        TELEMETRY_COMMAND,
    }
    task5_runtime_capabilities = {
        "semantic-vector-receipt-v2",
        "node-observability-v2",
        "expression-projection-v1",
    }
    assert {
        mapped[capability_id]["future_test_node"]
        for capability_id in task5_runtime_capabilities
    } == {TASK5_RUNTIME_SEMANTIC_COMMAND}
    assert mapped["aesem2-read-compatibility"]["future_test_node"] == TASK5_AESEM2_COMMAND
    assert mapped["aesem3-current-write"]["future_test_node"] == TASK5_CURRENT_WRITE_COMMAND
    assert mapped["finite-domain-migration"]["future_test_node"] == TASK5_FINITE_COMMAND
    assert mapped["migration-preimage-backup"]["future_test_node"] == TASK5_BACKUP_COMMAND
    assert mapped["formula-upgrade-proof"]["future_test_node"] == TASK5_FORMULA_COMMAND
    assert mapped["paired-atomic-commit"]["future_test_node"] == TASK6_ATOMICITY_COMMAND
    for capability_id in {
        "event-deduplication", "stale-base-rejection", "identity-conflict-rejection",
    }:
        assert mapped[capability_id]["future_test_node"] == TASK6_DEDUPE_COMMAND
    for capability_id in {"evidence-canonical-codec", "independent-semantic-cursor"}:
        assert mapped[capability_id]["future_test_node"] == TASK7_EVOLUTION_COMMAND
    for capability_id in {"relation-private-evidence", "persona-global-mood"}:
        assert mapped[capability_id]["future_test_node"] == TASK7_RELATION_COMMAND
    for receipt, command in (
        (TASK5_AESEM3_RECEIPT, TASK5_AESEM3_COMMAND),
        (TASK5_SQL_TYPE_RECEIPT, TASK5_SQL_TYPE_COMMAND),
        (TASK5_SQL_SIZE_RECEIPT, TASK5_SQL_SIZE_COMMAND),
        (TASK5_CREATOR_TYPE_RECEIPT, TASK5_CREATOR_TYPE_COMMAND),
        (TASK5_ROWLESS_RECEIPT, TASK5_ROWLESS_COMMAND),
        (TASK5_STORE_PACKAGE_RECEIPT, TASK5_STORE_PACKAGE_COMMAND),
        (TASK5_PLATFORM_PACKAGE_RECEIPT, TASK5_PLATFORM_PACKAGE_COMMAND),
        (TASK5_CAS_RECEIPT, TASK5_CAS_COMMAND),
        (TASK5_SQL_ROLLBACK_RECEIPT, TASK5_SQL_ROLLBACK_COMMAND),
        (TASK5_REOPEN_RECEIPT, TASK5_REOPEN_COMMAND),
    ):
        _validate_execution_receipt(
            receipt,
            command,
            {ATOMIC_SEMANTIC_MIGRATION_TARGET_COMMIT},
        )
    _validate_execution_receipt(
        ERROR_ATOMICITY_RECEIPT,
        ERROR_ATOMICITY_COMMAND,
        {DYNAMICS_TARGET_COMMIT},
    )
    _validate_execution_receipt(
        RUNTIME_CHECK_RECEIPT,
        RUNTIME_CHECK_COMMAND,
        {DYNAMICS_TARGET_COMMIT},
        exact_test=False,
    )
    assert gate["status"] == "NO_GO"
    assert gate["unmapped_capability_count"] == 14


def test_release_gate_rejects_malformed_unpinned_or_unearned_mappings():
    provenance, parity, _ = _load_and_validate()
    universe = _validate_provenance(provenance)
    unknown = copy.deepcopy(parity)
    unknown["capabilities"][0]["disposition"] = "UNKNOWN"
    with pytest.raises(AssertionError):
        _validate_parity(unknown, universe)
    empty_evidence = copy.deepcopy(parity)
    row = next(row for row in empty_evidence["capabilities"] if row["id"] == "evidence-slot-order")
    row["evidence"] = []
    with pytest.raises(AssertionError):
        _validate_parity(empty_evidence, universe)
    unpinned = copy.deepcopy(parity)
    row = next(row for row in unpinned["capabilities"] if row["id"] == "evidence-slot-order")
    for ref in row["target_refs"]:
        ref["commit"] = "0" * 40
    row["evidence"][0]["target_commit"] = "0" * 40
    with pytest.raises(AssertionError):
        _validate_parity(unpinned, universe)
    inexact_node = copy.deepcopy(parity)
    row = next(row for row in inexact_node["capabilities"] if row["id"] == "evidence-slot-order")
    row["future_test_node"] = "pytest broad-suite"
    with pytest.raises(AssertionError):
        _validate_parity(inexact_node, universe)
    tampered_receipt = copy.deepcopy(parity)
    row = next(row for row in tampered_receipt["capabilities"] if row["id"] == "evidence-slot-order")
    row["evidence"][0]["sha256"] = "0" * 64
    with pytest.raises(AssertionError):
        _validate_parity(tampered_receipt, universe)


def test_native_telemetry_sealing_authority_is_crate_private():
    runtime_lib = (ROOT / "crates/ae-runtime/src/lib.rs").read_text("utf-8")
    telemetry_source = (
        ROOT / "crates/ae-runtime/src/semantic_telemetry_v1.rs"
    ).read_text("utf-8")

    assert "mod semantic_telemetry_v1;" in runtime_lib
    assert "pub mod semantic_telemetry_v1;" not in runtime_lib
    assert "pub(crate) fn prepare_native_telemetry_v1(" in telemetry_source
    assert "\npub fn prepare_native_telemetry_v1(" not in telemetry_source


def test_frozen_source_checkout_is_clean():
    assert _git("status", "--short", cwd=SOURCE_ROOT) == ""


def test_provenance_custody_is_immutably_locked():
    provenance_path = "model/emotion-matrix-provenance-v1.json"
    assert _commit_exists(PROVENANCE_FREEZE_COMMIT)
    assert _git("rev-parse", f"{PROVENANCE_FREEZE_COMMIT}:{provenance_path}") == PROVENANCE_BLOB_OBJECT
    assert _git("rev-parse", f"HEAD:{provenance_path}") == PROVENANCE_BLOB_OBJECT
    later_changes = _git("log", "--format=%H", f"{PROVENANCE_FREEZE_COMMIT}..HEAD", "--", provenance_path)
    assert later_changes == ""
    assert _git("log", "-1", "--format=%H", "--", provenance_path) == PROVENANCE_FREEZE_COMMIT
