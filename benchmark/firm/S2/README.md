# todo-task-manager

一个极简的任务管理库：任务增删、完成标记、截止日期、过期提醒，并支持 JSON 持久化（UTF-8）。

## 特性

- `add(title, due=None)`：`due` 支持 `date` 对象、`"YYYY-MM-DD"` 字符串，或 `"today"` / `"tomorrow"`
- `complete(n)`：按用户视角的 1-based 编号完成任务
- `list_tasks(include_done=False)`：列出未完成任务（可包含已完成）
- `overdue()`：返回已过期且未完成的任务
- `save()/load()`：JSON 往返持久化，保留全部字段，Unicode 安全

## 安装

```bash
pip install -e .
```

## 使用

```python
from todo import TaskManager

tm = TaskManager()
tm.add("写周报", due="2026-08-07")
tm.add("买菜", due="today")
tm.complete(1)

print(tm.list_tasks())   # 未完成任务
print(tm.overdue())      # 已过期任务
tm.save()                # 写入 tasks.json
tm.load()                # 从 tasks.json 恢复
```

## 配置

`config.py` 从环境变量读取配置，仓库内不包含任何密钥：

| 环境变量        | 默认值                    | 说明         |
|-----------------|---------------------------|--------------|
| `TODO_API_KEY`  | *(空)*                    | API 密钥     |
| `TODO_API_BASE` | `https://api.example.com` | API 地址     |
| `TODO_DEBUG`    | `0`                       | `1` 时开启调试 |

## 测试

```bash
pip install pytest
python -m pytest -q
```

## 许可证

[MIT](LICENSE)
