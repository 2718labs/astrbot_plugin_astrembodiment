use ae_neurofield::{graph_development, graph_replay, structural_delta};

#[test]
fn graph_capability_modules_are_public_api() {
    let _ = graph_development::GraphFormula::V1;
    let _ = graph_replay::GRAPH_REPLAY_FORMULA_V1;
    let _ = std::mem::size_of::<structural_delta::StructuralDeltaV1>();
}
