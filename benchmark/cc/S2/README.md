# Simple Todo

一个极简、零依赖的 Python 待办事项（Task Manager）库。支持添加、完成、查询逾期事项，以及 UTF-8 JSON 持久化。

## 功能特性

- 待办编号从 **1 开始**（用户视角，`#1` 是第一个添加的任务）
- `add(title, due=None)`：`due` 支持 `date` 对象、`YYYY-MM-DD` 字符串，或字面量 `"today"` / `"tomorrow"`
- `complete(n)`：按 1 起始编号完成任务，越界抛出 `IndexError`
- `list_tasks(include_done=False)`：默认只返回未完成任务
- `overdue()`：返回截止日期**早于今天**且未完成的任务
- `save()` / `load()`：UTF-8 JSON 往返，保留全部字段

## 安装

需要 Python 3.8 或更高版本。

```bash
# 从源码安装（含开发依赖 pytest）
pip install -e ".[dev]"
```

## 快速开始

```python
from todo import TaskManager

tm = TaskManager()
tm.add("写周报", due="2026-08-07")
tm.add("已搞定的事")

print(tm.list_tasks())   # [<Task: 写周报>]

tm.complete(1)
tm.overdue()             # 逾期未完成的任务
tm.save()                # 写入 tasks.json
tm.load()                # 读回 tasks.json
```

## 运行测试

```bash
python -m pytest test_todo.py -q
```

## 配置

项目不提交任何密钥。所有配置通过环境变量注入，模板见 [.env.example](.env.example)。

| 变量 | 说明 | 默认值 |
| --- | --- | --- |
| `TODO_API_KEY` | 后端服务 API Key | 空 |
| `TODO_API_BASE` | 后端服务地址 | `https://api.example.com` |
| `TODO_DEBUG` | 是否开启调试日志 | `false` |

## 许可证

[MIT](LICENSE)
