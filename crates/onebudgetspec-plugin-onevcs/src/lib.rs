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

pub mod history;
pub mod reporter;

mod ancestry;
