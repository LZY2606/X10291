# 命名空间罗盘：OPC UA NodeSet 合并、引用闭包与模型差异

离线案例包。工业建模团队把多份 NodeSet XML 与 namespace URI 映射导入系统，
组装类型、实例与引用图，并比较两个模型版本。所有材料都在本目录，无外部依赖。

## 材料（原样保留层）

| 文件 | 模型 | 触发的边界情形 |
| --- | --- | --- |
| `core_types.xml` | compass/core 1.0.0 | 未知扩展（VendorCalibration）、量值（23.5, Unit=degC, Basis=sensor-local） |
| `line_a.xml` | compass/lineA 1.0.0 | alias 循环（LoopA↔LoopB）、逆向引用（IsForward=false 的 HasComponent）、跨文件引用 ns=3;i=7002 |
| `line_b_v1.xml` | compass/lineB 1.0.0 | namespace 重排：core 在 line_a 中是 index 2，在本文件是 index 1 |
| `line_b_v1_reindexed.xml` | compass/lineB 1.0.0 | 与 v1 语义相同、仅 index 互换，用于证明 index 只在单文件内有意义 |
| `line_b_v2.xml` | compass/lineB 2.0.0 | 依赖版本冲突（要 core 2.0.0，库里只有 1.0.0）、重复 NodeId（ns=2;i=7004 两次）、删除 7002 后 lineA 悬空引用、引用不存在的 ns=1;i=2999、不可换算单位 furlong/fortnight |

## 叙事顺序（重放记录）

1. **初始导入**：core_types + line_a + line_b_v1 → 基线版本。审阅者核对原 XML 节点、
   namespace table、NodeId、BrowseName、reference、alias、model dependency、
   未识别扩展均保留。
2. **参数修订**：alarm_threshold 由 80.0 改为 90.0（degC）。只有依赖该参数的推导
   （报警诊断）重算，结构层原样复用并给出“为何仍有效”的说明。
3. **局部源数据改动**：line_b_v1 替换为 line_b_v2。只影响 lineB 及其依赖者：
   依赖版本冲突、重复 NodeId、悬空引用、不可换算单位逐一落档。
4. **互相冲突的人工决定**：aoi 与 ren 从同一版本出发，分别选择重复 NodeId
   7004 的不同候选。第二次保存产生可解决冲突；未选候选与分支都保留。
5. **第二分支**：`branch-index-check` 用 line_b_v1_reindexed 重建，语义差异为空，
   证明数字 namespace index 只在单文件内有意义，合并按 URI 重映射。

## 判断依据（机器可读事件中的 basis 字段）

- 数字 namespace index 只在单文件内有意义，合并按 URI 重映射。
- inverse reference 与 forward reference 规范化但不制造缺边。
- 依赖缺失保留 unresolved 节点。

## 领域量约定

所有量值携带单位（Unit）与坐标基准（Basis）。导入时保留原始表示（Value 原文、
扩展原文），派生层才做规范化（degC/degF→K，bar→Pa，m→m）。无法换算的值
（furlong/fortnight）留在原位置并附诊断，不为画图丢弃。
