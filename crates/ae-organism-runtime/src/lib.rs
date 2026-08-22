#![forbid(unsafe_code)]

//! Retired R7 organism ABI facade.
//!
//! Atomic semantic projection, typed pre-output assembly, private wire minting,
//! and semantic state commit are private implementation details of `ae-runtime`.
//! This crate intentionally exports no compatibility types.
//!
//! The production facade is intentionally absent after compatibility transfer.
//!
//! ```compile_fail
//! use ae_organism_runtime::{
//!     NativeProjectionPayloadProducerV1, PrivateProjectionPayloadWireV1,
//! };
//!
//! let _ = std::any::TypeId::of::<NativeProjectionPayloadProducerV1>();
//! let _ = std::any::TypeId::of::<PrivateProjectionPayloadWireV1>();
//! ```
