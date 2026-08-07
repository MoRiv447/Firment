# Todo Manager

一个极简任务管理库，提供任务的新增、完成、逾期检查与 JSON 持久化。

> 注意：本仓库是教学/评测项目，`todo.py` 中**有意埋入了 3 个 bug**（详见 `test_todo.py` 中的注释），
> 因此当前测试套件预期有 3 个失败。修复它们属于练习内容，不影响 API 使用方式。

## 功能

- `add(title, due=None)` — 新增任务；`due` 接受 `date`、`YYYY-MM-DD` 字符串或 `"today"` / `"tomorrow"` 字面量
- `complete(n)` — 按用户视角的 1 起始编号完成任务
- `list_tasks(include_done=False)` — 列出任务，默认仅未完成
- `overdue()` — 列出已逾期且未完成的任务
- `save()` / `load()` — UTF-8 JSON 持久化，保留全部字段
- 优先级（`next()` 等）需求见 [FEATURE_BRIEF.md](FEATURE_BRIEF.md)

## 安装

```bash
git clone <repo-url>
cd todo-manager
pip install -e .
```

## 使用示例

```python
from todo import TaskManager

tm = TaskManager()
tm.add("写周报", due="2026-08-07")
tm.add("回复邮件", due="today")
tm.complete(1)
print(tm.list_tasks())
tm.save()  # 写入 tasks.json
tm.load()  # 读回
```

## 运行测试

```bash
python -m pytest test_todo.py -q
```

评测方使用同一命令；当前基准为 3 失败、2 通过。

## 配置

配置通过环境变量读取（见 [config.py](config.py)），仓库中不提交任何密钥：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `TODO_API_KEY` | 空 | API 密钥 |
| `TODO_API_BASE` | `https://api.example.com` | API 地址 |
| `TODO_DEBUG` | `false` | 是否开启调试 |

复制 `.env.example` 为 `.env`，或直接在 shell 中 `export`（`.env` 已被 git 忽略）。

## 目录结构

```
todo.py           核心库
test_todo.py      参考测试套件
config.py         环境变量配置
FEATURE_BRIEF.md  功能需求说明
pyproject.toml    打包与测试配置
```

## 许可证

MIT — 见 [LICENSE](LICENSE)。
