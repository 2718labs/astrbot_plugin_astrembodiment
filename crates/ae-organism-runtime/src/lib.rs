#![forbid(unsafe_code)]

//! Legacy R7 organism ABI facade.
//!
//! Atomic semantic projection, typed pre-output assembly, private wire minting,
//! and semantic state commit are owned by `ae-runtime`. This crate deliberately
//! forwards only compatibility types for callers that have not yet moved their
//! import paths.

pub use ae_runtime::r7::{
    BoundedProjectionReferencesV1, NativeProjectionPayloadIngressV1,
    NativeProjectionPayloadProducerErrorV1, NativeProjectionPayloadProducerInputV1,
    NativeProjectionPayloadProducerV1, NativeProjectionUpdateV1, OrganismRuntimeErrorV1,
    PrivateProjectionPayloadWireErrorV1, PrivateProjectionPayloadWireV1,
};
