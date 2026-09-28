//! 命名空间罗盘 (compass) — OPC UA NodeSet 合并、引用闭包与模型差异。
//!
//! 核心语义：
//! - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
//! - inverse/forward 引用规范化但不制造缺边；
//! - 依赖缺失保留 unresolved 节点；
//! - 多解/不确定状态保留全部合法候选，不为单一答案吞掉分歧。

pub mod domain;
pub mod xmlutil;
pub mod parse;
pub mod merge;
pub mod engine;
pub mod events;
pub mod store;
pub mod export;
pub mod web;
