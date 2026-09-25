pub mod diff;
pub mod engine;
pub mod events;
pub mod hash;
pub mod ids;
pub mod model;
pub mod quantities;
pub mod xml;

pub const RULE_VERSION: &str = "rules-2026.09.1";

/// OPC UA namespace zero (implicit in every NodeSet file).
pub const NS0_URI: &str = "http://opcfoundation.org/UA/";
/// Namespace of Compass domain extensions (PhysicalQuantity, CoordinateReference).
pub const COMPASS_EXT_URI: &str = "http://compass.example/opcua/extensions";
