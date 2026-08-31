//! Red-team support modules: the deterministic mutation engine, the crash
//! oracle and the structured findings schema. The `redteam` tool
//! (`tools/redteam.rs`) orchestrates them; everything here is pure and
//! unit-testable without hardware — the same split that made `observe` and
//! `forensic` trustworthy.

pub mod mutate;
