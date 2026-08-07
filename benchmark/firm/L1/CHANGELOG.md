# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 风格。日期格式为 YYYY-MM-DD。

## [Unreleased] - 2026-08-06

### 修复

- **修复 `parse_due("tomorrow")` 的 off-by-one 错误**：此前返回今天，现返回 `date.today() + 1 天`（`todo.py`）。
- **修复 `complete` 的 0-based 索引错误**：此前 `complete(1)` 完成的是第二个任务；现按用户视角 1-based 处理，越界时仍抛 `IndexError`（`todo.py`）。
- **修复 save/load 完成状态丢失**：`to_dict` 此前把完成标志写为 `is_done`，而 `from_dict` 读取 `done`，导致已完成任务 round-trip 后变回未完成；现统一使用 `done` 键（`todo.py`）。

### 新增

- **优先级能力（FEATURE_BRIEF.md）**：
  - `add(title, due=None, priority="medium")` 支持 `"high" | "medium" | "low"`，非法值抛 `ValueError`。
  - `list_tasks` 在保持 `include_done` 语义不变的前提下，按优先级排序（high > medium > low），同优先级保持添加顺序。
  - priority 参与 JSON 持久化 round-trip；读取旧格式文件（无 priority 字段）时默认 `"medium"`。
  - 新增 `next()`：返回未完成且按上述排序最靠前的任务，无未完成时返回 `None`。
- 测试套件同步更新：新增优先级校验、排序稳定性、`include_done` 排序、`next()`、优先级 round-trip 及旧文件兼容用例（`test_todo.py`）。

### 变更

- **`complete` 重命名为 `mark_done`**（语义与 1-based 索引、`IndexError` 行为不变）；保留 `complete` 作为向后兼容的别名（`todo.py`）。
