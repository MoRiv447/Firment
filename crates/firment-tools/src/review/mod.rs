//! Review capabilities that need the tool layer (plan §4).
//!
//! The vocabulary lives in `firment_core::review` — the `Finding`/`ReviewReport` shape
//! every capability reports in. What lives here is what needs the *tools*: the evidence
//! collector reads the HIL replay log, and the local rule layer for the static review
//! (§4-C) will read the source tree.

pub mod evidence;
pub mod rules;
pub mod walk;
