# 命名空间罗盘：OPC UA NodeSet 合并、引用闭包与模型差异

面向工业建模团队的离线案例包：导入多份 NodeSet XML 与 namespace URI 映射，
组装类型、实例和引用图，并比较两个模型版本。

## 材料

- `01_base_types.xml` — 机械类型模型（`http://example.org/machinery/`），
  依赖 Opc.Ua 核心模型 1.05.02；含未知扩展 `VendorExtension` 与带单位
  （`EngineeringUnits="degC"`）和坐标基准（`CoordinateBasis`）的量。
- `02_devices.xml` — 设备实例文件。namespace 表故意重排（自有 URI 在
  index 2，machinery 在 index 3），含 alias 循环（`LoopA`↔`LoopB`）、
  仅逆向声明的引用、无法转换的值 `Value="N/A"`。
- `03_devices_v2.xml` — 一次参数修订（`FlowRate` 12.5 → 15.0 m3/h）叠加
  一次只影响局部依赖的源数据改动：删除 `Pump1`（使 `Conveyor1` 的
  `HasComponent` 悬空），并新增与基线重复的全局 NodeId
  （`nsu=http://example.org/machinery/;i=1001`）。
- `04_deps_conflict.xml` — 遗留模型，要求 Opc.Ua 1.04.00（与基线 1.05.02
  冲突）并依赖不存在的 `http://example.org/missing/`（保留为未解析节点）。
- `expected/events.json` — 机器可读的判定依据（每条判断对应的规则与检查点）。

## 叙事顺序

1. 初始导入 `01`+`02`：核对原 XML 节点、namespace table、NodeId、
   BrowseName、reference、alias、model dependency 与未识别扩展均保留。
2. 导入 `04`：触发依赖版本冲突与未解析依赖。
3. 用 `03` 替换 `02`：参数修订 + 局部源数据改动，触发重复 NodeId 与
   删除后悬空引用；重算只传播到受影响工件。
4. 两位使用者从同一版本对依赖版本冲突做出互相冲突的人工决定：
   系统产生可解决的冲突，两个候选都保留。
5. 复核路径：namespace 映射 → 类型层次 → 引用图 → 未解析依赖 →
   语义差异，每个值可追到源 XML 字节范围；回到任意旧事件恢复当时结论。
