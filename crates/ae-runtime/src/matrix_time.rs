//! Public runtime facade for the pure, deterministic matrix-time formula.
//!
//! The Store remains the sole authority for phase, epoch and anchor inputs;
//! this module only keeps downstream callers on the shared core formula.

pub use ae_semantic_core::{
    advance_matrix_time_v1, matrix_time_formula_digest_v1, MatrixSleepPhaseV1, MatrixTimeAdvanceV1,
    MatrixTimeEpochV1, MatrixTimeInputV1, MATRIX_TIME_FORMULA_V1, MATRIX_TIME_MAX_ELAPSED_MS,
    MATRIX_TIME_MAX_STEPS_PER_EVENT, MATRIX_TIME_QUANTUM_MS,
};
