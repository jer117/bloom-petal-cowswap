//! Shared, typed infrastructure for the CoW Swap Petal's route components.
//!
//! Everything here is domain infrastructure reused by several route files:
//! chain/network configuration, the GPv2 EIP-712 order encoding, appData
//! construction, a bounded HTTP adapter for `api.cow.fi`, strict parameter
//! grammars, and the draft store model. Nothing in this crate inspects route
//! identity; route files pass typed values in.

pub mod cow;
