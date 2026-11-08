//! Remote transport for onebudgetspec reports, over onevcs and local git.
//!
//! Budget semantics stay in `onebudgetspec-core`, whose wire types these functions take
//! and return unchanged. This crate adds exactly two transports:
//!
//! - [`reporter`] answers a `PublishRequest` with a `DeliveryResult`, keeping one standing
//!   comment per destination and key.
//! - [`history`] reads one saved `RollingHistory` from a base-branch workflow run's
//!   artifact, proving its provenance with host facts and local git ancestry.
//!
//! A transport failure is an answer rather than a change to a budget verdict: neither
//! module reads or alters one.
//!
//! # Compatibility
//!
//! This crate requires `onebudgetspec-core` 0.2. Because its public API takes and returns
//! core types, that is a breaking change: a consumer still on `onebudgetspec-core` 0.1 must
//! move to 0.2 together with this crate. Release 0.1.2 already carried that requirement
//! though published as a patch; 0.2.0 is the release that marks it as breaking.

pub mod history;
pub mod reporter;

mod ancestry;
