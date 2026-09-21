//! Canonical, compare-and-swap structural graph deltas.
//!
//! A delta is always applied to a private clone.  Invalid, stale, or otherwise
//! rejected deltas therefore cannot leave a caller-owned graph partially
//! changed.

use std::{collections::BTreeSet, fmt};

use ae_contracts::Digest;
use serde::{
    de::{Error as _, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};

use crate::{
    graph_digest,
    graph_replay::{graph_admission_profile_digest, GraphAdmissionProfileV1},
    SparseGraph, Synapse, EDGE_CAPACITY, NEURON_SLOTS,
};

const V1_OPERATOR_TYPE_COUNT: u8 = 4;
const V1_DELAY_CLASS_COUNT: u8 = 8;

/// Maximum edit operations admitted by one persisted v1 structural delta.
///
/// The bound matches the largest 4,096-slot v1 region, so one transition can
/// replace a complete region but cannot request an unbounded graph rewrite.
pub const MAX_OPERATIONS_PER_DELTA_V1: usize = 4_096;

/// A complete, v1 structural transition over a sparse graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralDeltaV1 {
    pub base_revision: u64,
    pub base_graph_digest: Digest,
    pub delta_sequence: u64,
    pub rule_digest: Digest,
    #[serde(deserialize_with = "deserialize_operations_v1")]
    pub operations: Vec<EdgeOperationV1>,
    pub after_graph_digest: Digest,
}

/// Closed v1 edit operations.  `Add` and `Update` carry the complete edge so
/// all graph bytes are explicit and no implicit merge rule exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EdgeOperationV1 {
    #[serde(rename = "Add")]
    Add {
        #[serde(rename = "source")]
        source: u32,
        #[serde(rename = "edge")]
        edge: Synapse,
    },
    #[serde(rename = "Update")]
    Update {
        #[serde(rename = "source")]
        source: u32,
        #[serde(rename = "edge")]
        edge: Synapse,
    },
    #[serde(rename = "Remove")]
    Remove {
        #[serde(rename = "source")]
        source: u32,
        #[serde(rename = "target")]
        target: u32,
    },
}

fn deserialize_operations_v1<'de, D>(deserializer: D) -> Result<Vec<EdgeOperationV1>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OperationsVisitor;

    impl<'de> Visitor<'de> for OperationsVisitor {
        type Value = Vec<EdgeOperationV1>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "at most {MAX_OPERATIONS_PER_DELTA_V1} v1 edge operations"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let size_hint = sequence.size_hint().unwrap_or(0);
            if size_hint > MAX_OPERATIONS_PER_DELTA_V1 {
                return Err(A::Error::custom("too many v1 structural delta operations"));
            }

            let mut operations = Vec::with_capacity(size_hint.min(MAX_OPERATIONS_PER_DELTA_V1));
            while let Some(operation) = sequence.next_element()? {
                if operations.len() == MAX_OPERATIONS_PER_DELTA_V1 {
                    return Err(A::Error::custom("too many v1 structural delta operations"));
                }
                operations.push(operation);
            }
            Ok(operations)
        }
    }

    deserializer.deserialize_seq(OperationsVisitor)
}

impl EdgeOperationV1 {
    fn source(&self) -> u32 {
        match self {
            Self::Add { source, .. }
            | Self::Update { source, .. }
            | Self::Remove { source, .. } => *source,
        }
    }

    fn target(&self) -> u32 {
        match self {
            Self::Add { edge, .. } | Self::Update { edge, .. } => edge.target,
            Self::Remove { target, .. } => *target,
        }
    }

    fn operation_rank(&self) -> u8 {
        match self {
            Self::Add { .. } => 0,
            Self::Update { .. } => 1,
            Self::Remove { .. } => 2,
        }
    }

    fn edge(&self) -> Option<Synapse> {
        match self {
            Self::Add { edge, .. } | Self::Update { edge, .. } => Some(*edge),
            Self::Remove { .. } => None,
        }
    }
}

/// Stable rejection classifications for structural delta validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaError {
    TooManyOperations,
    AdmissionProfileMismatch,
    StaleRevision,
    StaleGraphDigest,
    CurrentGraphInvalid,
    OperationsNotCanonical,
    DuplicateEdgeOperation,
    SourceOutOfBounds,
    TargetOutOfBounds,
    UnknownOperator,
    UnknownDelayClass,
    EdgeAlreadyExists,
    EdgeMissing,
    EdgeCardinalityUnderflow,
    EdgeCapacityExceeded,
    AfterGraphDigestMismatch,
}

/// Applies one fully-specified structural delta as a revision-and-digest CAS.
///
/// The caller supplies the expected revision and graph digest explicitly.  The
/// delta must bind to those expected values and they must still match the
/// supplied current revision and graph.  No input graph is mutated on any
/// outcome; success returns the entire next graph.
pub fn apply_delta(
    expected_revision: u64,
    expected_graph_digest: &Digest,
    current_revision: u64,
    current_graph: &SparseGraph,
    delta: &StructuralDeltaV1,
) -> Result<SparseGraph, DeltaError> {
    if delta.operations.len() > MAX_OPERATIONS_PER_DELTA_V1 {
        return Err(DeltaError::TooManyOperations);
    }
    let expected_rule_digest = graph_admission_profile_digest(GraphAdmissionProfileV1::ClosedV1)
        .map_err(|_| DeltaError::AdmissionProfileMismatch)?;
    apply_delta_with_rule_digest(
        expected_revision,
        expected_graph_digest,
        current_revision,
        current_graph,
        delta,
        &expected_rule_digest,
    )
}

pub(crate) fn apply_delta_with_rule_digest(
    expected_revision: u64,
    expected_graph_digest: &Digest,
    current_revision: u64,
    current_graph: &SparseGraph,
    delta: &StructuralDeltaV1,
    expected_rule_digest: &Digest,
) -> Result<SparseGraph, DeltaError> {
    if delta.operations.len() > MAX_OPERATIONS_PER_DELTA_V1 {
        return Err(DeltaError::TooManyOperations);
    }
    if delta.rule_digest != *expected_rule_digest {
        return Err(DeltaError::AdmissionProfileMismatch);
    }
    if expected_revision != current_revision || delta.base_revision != expected_revision {
        return Err(DeltaError::StaleRevision);
    }

    let current_digest = graph_digest(current_graph);
    if delta.base_graph_digest != *expected_graph_digest || current_digest != *expected_graph_digest
    {
        return Err(DeltaError::StaleGraphDigest);
    }

    validate_graph(current_graph)?;
    validate_operations(&delta.operations)?;
    let projected_edge_count =
        preflight_edge_cardinality(current_graph.edges.len(), &delta.operations)?;

    let mut rows = rows_from_graph(current_graph);
    for operation in &delta.operations {
        apply_operation(&mut rows, operation)?;
    }

    debug_assert_eq!(
        rows.iter().map(Vec::len).sum::<usize>(),
        projected_edge_count
    );
    let next_graph = graph_from_rows(rows);
    validate_graph(&next_graph)?;
    if graph_digest(&next_graph) != delta.after_graph_digest {
        return Err(DeltaError::AfterGraphDigestMismatch);
    }
    Ok(next_graph)
}

fn preflight_edge_cardinality(
    current_edge_count: usize,
    operations: &[EdgeOperationV1],
) -> Result<usize, DeltaError> {
    let mut additions = 0usize;
    let mut removals = 0usize;
    for operation in operations {
        match operation {
            EdgeOperationV1::Add { .. } => {
                additions = additions
                    .checked_add(1)
                    .ok_or(DeltaError::EdgeCapacityExceeded)?;
            }
            EdgeOperationV1::Remove { .. } => {
                removals = removals
                    .checked_add(1)
                    .ok_or(DeltaError::EdgeCardinalityUnderflow)?;
            }
            EdgeOperationV1::Update { .. } => {}
        }
    }

    let projected = current_edge_count
        .checked_sub(removals)
        .ok_or(DeltaError::EdgeCardinalityUnderflow)?
        .checked_add(additions)
        .ok_or(DeltaError::EdgeCapacityExceeded)?;
    if projected > EDGE_CAPACITY {
        return Err(DeltaError::EdgeCapacityExceeded);
    }
    Ok(projected)
}

fn validate_graph(graph: &SparseGraph) -> Result<(), DeltaError> {
    if !graph.validate() {
        return Err(DeltaError::CurrentGraphInvalid);
    }
    for edge in &graph.edges {
        validate_synapse(edge)?;
    }
    Ok(())
}

fn validate_operations(operations: &[EdgeOperationV1]) -> Result<(), DeltaError> {
    let mut previous_key = None;
    let mut pairs = BTreeSet::new();

    for operation in operations {
        let source = operation.source();
        let target = operation.target();
        if source as usize >= NEURON_SLOTS {
            return Err(DeltaError::SourceOutOfBounds);
        }
        if target as usize >= NEURON_SLOTS {
            return Err(DeltaError::TargetOutOfBounds);
        }
        if let Some(edge) = operation.edge() {
            validate_synapse(&edge)?;
        }

        if !pairs.insert((source, target)) {
            return Err(DeltaError::DuplicateEdgeOperation);
        }
        let key = (source, target, operation.operation_rank());
        if previous_key.is_some_and(|previous| previous > key) {
            return Err(DeltaError::OperationsNotCanonical);
        }
        previous_key = Some(key);
    }
    Ok(())
}

fn validate_synapse(edge: &Synapse) -> Result<(), DeltaError> {
    if edge.operator_id >= V1_OPERATOR_TYPE_COUNT {
        return Err(DeltaError::UnknownOperator);
    }
    if edge.delay_class >= V1_DELAY_CLASS_COUNT {
        return Err(DeltaError::UnknownDelayClass);
    }
    Ok(())
}

fn rows_from_graph(graph: &SparseGraph) -> Vec<Vec<Synapse>> {
    (0..NEURON_SLOTS)
        .map(|source| {
            let start = graph.row_offsets[source] as usize;
            let end = graph.row_offsets[source + 1] as usize;
            graph.edges[start..end].to_vec()
        })
        .collect()
}

fn apply_operation(
    rows: &mut [Vec<Synapse>],
    operation: &EdgeOperationV1,
) -> Result<(), DeltaError> {
    let row = &mut rows[operation.source() as usize];
    let target = operation.target();
    let index = row.binary_search_by_key(&target, |edge| edge.target);

    match operation {
        EdgeOperationV1::Add { edge, .. } => match index {
            Ok(_) => Err(DeltaError::EdgeAlreadyExists),
            Err(index) => {
                row.insert(index, *edge);
                Ok(())
            }
        },
        EdgeOperationV1::Update { edge, .. } => match index {
            Ok(index) => {
                row[index] = *edge;
                Ok(())
            }
            Err(_) => Err(DeltaError::EdgeMissing),
        },
        EdgeOperationV1::Remove { .. } => match index {
            Ok(index) => {
                row.remove(index);
                Ok(())
            }
            Err(_) => Err(DeltaError::EdgeMissing),
        },
    }
}

fn graph_from_rows(rows: Vec<Vec<Synapse>>) -> SparseGraph {
    let mut graph = SparseGraph::empty();
    graph.row_offsets.clear();
    graph.row_offsets.push(0);
    for row in rows {
        graph.edges.extend(row);
        graph.row_offsets.push(graph.edges.len() as u32);
    }
    graph
}
