# 命名空间罗盘：OPC UA NodeSet 合并、引用闭包与模型差异（离线案例包）

仅使用仓库内材料；`cargo test --all-targets` 会逐场景重放。叙事：工业建模团队导入多份
NodeSet XML 与 namespace URI 映射，组装类型、实例和引用图，并比较两个模型版本。

## 阶段（不要拆成无关按钮——页面“短样例路径”按此顺序推进）

1. **初始导入 v1**：审阅者核对原 XML 节点、namespace table、NodeId、BrowseName、
   reference、alias、model dependency 与未识别扩展均保留（每个显示值带稳定身份与源 XML 范围）。
2. **参数修订**：`v2/params.xml` 只改一个变量的工程单位与数值（带单位/坐标基准）。
3. **只影响局部依赖的源数据改动**：`v2/local.xml` 只新增一个依赖 `ServerStatus` 的对象，
   用于验证依赖图传播只重算受影响部分。
4. **互相冲突的人工决定**：`v2/conflict.xml` 对同一全局 NodeId 给出两种类型定义，
   页面比较两个分支后必须保留未选候选。

## 边界组合（重放必须实际经过）

| 情形 | 材料 | 期望机器事件 |
| --- | --- | --- |
| namespace 重排 | `v1/base.xml` 与 `v1/line.xml` 同一 URI 出现在不同数字 index | `NS_INDEX_FILE_LOCAL` |
| alias 循环 | `v2/aliases_cycle.xml`（A→B→A） | `ALIAS_CYCLE` |
| 重复 NodeId | `v2/duplicate.xml` | `DUPLICATE_NODEID` |
| 逆向引用 | `v1/line.xml` 中 `IsForward="false"` | 规范化但不产生反向边 |
| 依赖版本冲突 | `v2/dep_conflict.xml` 要求同一模型两个版本 | `DEP_VERSION_CONFLICT` |
| 未知扩展 | 两份文件中的 `<VendorExtension>` | `UNKNOWN_EXTENSION`（原样保留） |
| 删除后悬空引用 | 重放时“删除” `v1/base.xml` 的删除事件 | `DANGLING_REFERENCE` + unresolved 占位 |

## 语义判据（机器可读 rationale）

- 数字 namespace index 只在单文件内有意义，合并按 URI 重映射。
- inverse reference 与 forward reference 规范化为同一条有向边，不制造缺边。
- 依赖缺失保留 unresolved 节点。
- 无法唯一决定时保留全部合法候选。

## 领域量

变量携带单位与坐标基准：`1200 mm @ MachineCoord`、`6 bar`、`36.5 degC` 可规范化到 SI；
`42 furlong` 无转换规则，留在原位置并附诊断，不允许为画图丢弃。
