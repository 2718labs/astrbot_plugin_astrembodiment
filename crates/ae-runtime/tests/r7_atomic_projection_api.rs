//! Desired external R7 surface: typed input only. This is intentionally added
//! before the implementation so the rejected callback/producer surface cannot
//! silently satisfy the acceptance test.

use ae_contracts::r7::CanonicalEvent;
use ae_runtime::r7::{AstrRuntime, R7PreOutputProjectionInputV1};

fn accepts_only_typed_input(
    runtime: &mut AstrRuntime,
    event: &CanonicalEvent,
    input: &R7PreOutputProjectionInputV1,
) {
    let _ = runtime.apply_user_stimulus_with_private_projection_wire_v1(event, input);
}

#[test]
fn desired_public_surface_compiles() {
    let _ = accepts_only_typed_input;
}
