use ae_attention::r7::assemble_load;
use ae_contracts::r7::EvidenceVector;
use ae_fixed::Fixed;

const REGION_LAYOUT: [(usize, usize); 9] = [
    (0, 2_048),
    (2_048, 2_048),
    (4_096, 1_024),
    (5_120, 2_048),
    (7_168, 2_048),
    (9_216, 1_024),
    (10_240, 4_096),
    (14_336, 1_024),
    (15_360, 1_024),
];

const PRIMARY: Fixed = Fixed::ONE;
const SECONDARY: Fixed = Fixed::from_raw(500_000);

#[derive(Clone, Copy)]
struct RouteCase {
    name: &'static str,
    set: fn(&mut EvidenceVector, Fixed),
    primary: usize,
    secondary: Option<usize>,
}

fn positive(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.positive = value;
}

fn affiliation(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.affiliation = value;
}

fn harm(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.harm = value;
}

fn boundary(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.boundary = value;
}

fn repair(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.repair = value;
}

fn repetition(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.repetition = value;
}

fn new_information(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.new_information = value;
}

fn constraint_instability(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.constraint_instability = value;
}

fn epistemic_conflict(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.epistemic_conflict = value;
}

fn self_responsibility(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.self_responsibility = value;
}

fn other_responsibility(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.other_responsibility = value;
}

fn hostility(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.hostility = value;
}

fn publicness(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.publicness = value;
}

fn engagement(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.engagement = value;
}

fn rejection(evidence: &mut EvidenceVector, value: Fixed) {
    evidence.rejection = value;
}

const CASES: [RouteCase; 15] = [
    RouteCase {
        name: "positive",
        set: positive,
        primary: 1,
        secondary: Some(8),
    },
    RouteCase {
        name: "affiliation",
        set: affiliation,
        primary: 1,
        secondary: Some(8),
    },
    RouteCase {
        name: "harm",
        set: harm,
        primary: 0,
        secondary: Some(5),
    },
    RouteCase {
        name: "boundary",
        set: boundary,
        primary: 4,
        secondary: Some(5),
    },
    RouteCase {
        name: "repair",
        set: repair,
        primary: 3,
        secondary: Some(8),
    },
    RouteCase {
        name: "repetition",
        set: repetition,
        primary: 2,
        secondary: Some(7),
    },
    RouteCase {
        name: "new_information",
        set: new_information,
        primary: 6,
        secondary: Some(2),
    },
    RouteCase {
        name: "constraint_instability",
        set: constraint_instability,
        primary: 2,
        secondary: Some(3),
    },
    RouteCase {
        name: "epistemic_conflict",
        set: epistemic_conflict,
        primary: 3,
        secondary: Some(7),
    },
    RouteCase {
        name: "self_responsibility",
        set: self_responsibility,
        primary: 3,
        secondary: Some(7),
    },
    RouteCase {
        name: "other_responsibility",
        set: other_responsibility,
        primary: 4,
        secondary: Some(7),
    },
    RouteCase {
        name: "hostility",
        set: hostility,
        primary: 5,
        secondary: Some(4),
    },
    RouteCase {
        name: "publicness",
        set: publicness,
        primary: 4,
        secondary: Some(7),
    },
    RouteCase {
        name: "engagement",
        set: engagement,
        primary: 8,
        secondary: Some(7),
    },
    RouteCase {
        name: "rejection",
        set: rejection,
        primary: 0,
        secondary: Some(4),
    },
];

fn expected_loads(case: RouteCase) -> [Fixed; REGION_LAYOUT.len()] {
    let mut expected = [Fixed::ZERO; REGION_LAYOUT.len()];
    expected[case.primary] = PRIMARY;
    if let Some(secondary) = case.secondary {
        expected[secondary] = SECONDARY;
    }
    expected
}

fn expected_nodes(loads: &[Fixed; REGION_LAYOUT.len()]) -> Vec<u32> {
    REGION_LAYOUT
        .iter()
        .enumerate()
        .filter(|(region, _)| loads[*region] != Fixed::ZERO)
        .flat_map(|(_, (start, count))| (*start..*start + *count).map(|node| node as u32))
        .collect()
}

fn region_for_node(node: u32) -> usize {
    let node = node as usize;
    REGION_LAYOUT
        .iter()
        .position(|(start, count)| (*start..*start + *count).contains(&node))
        .expect("test layout covers every canonical node")
}

#[test]
fn every_semantic_dimension_has_only_its_declared_region_loads() {
    for case in CASES {
        let mut evidence = EvidenceVector::default();
        (case.set)(&mut evidence, Fixed::ONE);

        let load = assemble_load(&evidence, 16_384);
        let expected_loads = expected_loads(case);

        assert_eq!(load.regional_loads, expected_loads, "{} regions", case.name);
        assert_eq!(
            load.active_nodes,
            expected_nodes(&expected_loads),
            "{} nodes",
            case.name
        );
        assert_eq!(
            load.node_loads.len(),
            load.active_nodes.len(),
            "{} pairs",
            case.name
        );
        for (&node, &node_load) in load.active_nodes.iter().zip(&load.node_loads) {
            assert_eq!(
                node_load,
                expected_loads[region_for_node(node)],
                "{} node {node}",
                case.name
            );
        }
    }
}

#[test]
fn zero_evidence_has_no_active_node_or_node_load() {
    let load = assemble_load(&EvidenceVector::default(), 16_384);

    assert_eq!(load.regional_loads, [Fixed::ZERO; REGION_LAYOUT.len()]);
    assert!(load.active_nodes.is_empty());
    assert!(load.node_loads.is_empty());
}
