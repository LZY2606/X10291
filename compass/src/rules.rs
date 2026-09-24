//! Machine-readable rule codes and their human-readable rationale.
//!
//! Every judgment emitted by merge/graph/propagation carries one of these
//! codes so the UI and replay archive can show *why* a result stands or must
//! be recomputed, without reparsing prose.

pub const RULESET_VERSION: &str = "compass-rules-3";

pub mod codes {
    pub const NS_URI_REMAP: &str = "NS_URI_REMAP";
    pub const NS_INDEX_UNDECLARED: &str = "NS_INDEX_UNDECLARED";
    pub const NS_REORDER: &str = "NS_REORDER";
    pub const ALIAS_CYCLE: &str = "ALIAS_CYCLE";
    pub const ALIAS_UNRESOLVED: &str = "ALIAS_UNRESOLVED";
    pub const DUPLICATE_NODE_ID: &str = "DUPLICATE_NODE_ID";
    pub const INVERSE_NORMALIZED: &str = "INVERSE_NORMALIZED";
    pub const FORWARD_NORMALIZED: &str = "FORWARD_NORMALIZED";
    pub const DEPENDENCY_VERSION_CONFLICT: &str = "DEPENDENCY_VERSION_CONFLICT";
    pub const DEPENDENCY_MISSING: &str = "DEPENDENCY_MISSING";
    pub const UNKNOWN_EXTENSION_RETAINED: &str = "UNKNOWN_EXTENSION_RETAINED";
    pub const DANGLING_REFERENCE: &str = "DANGLING_REFERENCE";
    pub const UNCONVERTIBLE_VALUE: &str = "UNCONVERTIBLE_VALUE";
    pub const COORD_FRAME_PRESERVED: &str = "COORD_FRAME_PRESERVED";
    pub const UNIT_NORMALIZED: &str = "UNIT_NORMALIZED";
    pub const REFERENCE_TYPE_UNRESOLVED: &str = "REFERENCE_TYPE_UNRESOLVED";
    pub const CANDIDATES_RETAINED: &str = "CANDIDATES_RETAINED";
}

pub fn rationale(code: &str) -> &'static str {
    match code {
        codes::NS_URI_REMAP => {
            "Numeric namespace indices are file-local. The merge resolves every \
index against that file's NamespaceUris table and re-keys nodes by URI, so a \
reorder cannot change identity."
        }
        codes::NS_REORDER => {
            "The same URI appears at a different file-local index in a new \
source; only the index mapping changed, node identities are URI-based."
        }
        codes::NS_INDEX_UNDECLARED => {
            "A NodeId references a file-local index with no Uri entry; the raw \
NodeId is retained and the node is unresolved."
        }
        codes::ALIAS_CYCLE => {
            "Alias expansion reached a cycle. No arbitrary target is chosen; \
all aliases on the cycle are retained as unresolved candidates."
        }
        codes::ALIAS_UNRESOLVED => {
            "The alias names a target with no definition; it is retained as an \
unresolved placeholder rather than dropped."
        }
        codes::DUPLICATE_NODE_ID => {
            "Multiple source nodes map to the same URI-keyed NodeId. Every \
candidate is retained with provenance; none is silently overwritten."
        }
        codes::INVERSE_NORMALIZED => {
            "An IsForward=false reference is normalized by swapping endpoints \
into forward direction. The inverse edge is not duplicated, so closure gains \
no fabricated edge."
        }
        codes::FORWARD_NORMALIZED => "A forward reference is kept as written.",
        codes::DEPENDENCY_VERSION_CONFLICT => {
            "Sources require different versions of the same model URI. Both \
requirements are retained; the ambiguity needs an explicit decision."
        }
        codes::DEPENDENCY_MISSING => {
            "A referenced namespace URI is not supplied by any source. The \
target is kept as an unresolved node, not removed."
        }
        codes::UNKNOWN_EXTENSION_RETAINED => {
            "An unrecognized extension subtree is preserved verbatim with its \
source span and surfaced as an anomaly."
        }
        codes::DANGLING_REFERENCE => {
            "A reference targets a node absent after the source changed; the \
edge is retained as dangling instead of being silently deleted."
        }
        codes::UNCONVERTIBLE_VALUE => {
            "The value has no supported unit conversion. Its original text \
stays attached to the node with this diagnostic."
        }
        codes::COORD_FRAME_PRESERVED => {
            "Coordinates are meaningful only relative to their declared frame; \
the raw representation is retained and no cross-frame merge is attempted."
        }
        codes::UNIT_NORMALIZED => "A supported unit conversion produced the canonical SI value.",
        codes::REFERENCE_TYPE_UNRESOLVED => {
            "The reference type could not be resolved; the raw edge is kept \
with the raw type text."
        }
        codes::CANDIDATES_RETAINED => {
            "More than one legal outcome exists; all candidates are retained \
pending an explicit decision."
        }
        _ => "No rationale recorded.",
    }
}
