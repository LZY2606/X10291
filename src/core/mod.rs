//! Core domain model for OPC UA NodeSet merge, reference closure and model diff.
//!
//! Design invariants (see `docs/semantics.md`):
//! 1. A numeric namespace index is meaningful only inside one file; merging
//!    re-maps by namespace URI.
//! 2. Inverse and forward references are normalised to a directed edge graph
//!    without fabricating missing edges.
//! 3. Missing dependencies are retained as unresolved nodes instead of being
//!    dropped.
//! 4. Ambiguous situations keep every legal candidate rather than collapsing
//!    to one answer.
//! 5. Raw XML nodes, namespace tables, NodeIds, BrowseNames, references,
//!    aliases, model dependencies and unknown extensions are all preserved.

pub mod model;
pub mod xml_parser;
pub mod identity;
pub mod ns;
pub mod alias;
pub mod refs;
pub mod merge;
pub mod derive;
pub mod units;
pub mod scenario;
pub mod diff;
pub mod events;
pub mod archive;

pub use model::*;
