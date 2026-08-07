# 新功能需求（用于 F 类用例）

为 TaskManager 增加**优先级**能力：

1. `add(title, due=None, priority="medium")`，priority 取值 `"high" | "medium" | "low"`，非法值抛 `ValueError`。
2. `list_tasks` 的返回顺序：未完成优先于已完成？不——保持现有 include_done 语义，但同组内按 priority 排序（high > medium > low），同优先级按添加顺序。
3. priority 必须参与 JSON 持久化的 round-trip。
4. 新增方法 `next()`：返回当前最该做的任务（未完成、按上述排序的第一个），没有则返回 None。
5. 同步更新/新增测试。

约束：不破坏现有公开 API 的既有行为（除排序规则新增外）。
