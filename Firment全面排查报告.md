# Firment 全面问题排查报告（2026-08-12）

> 范围：全部 4 个 crate（core / tools / tui / cli）· 方法：3 个并行审查代理逐 crate 深挖（clippy/测试已绿，聚焦静态检查抓不到的问题）
> 结果：**44 项（9 高 / 21 中 / 14 低）**，本轮已修复 8 项（含 4 项高），其余列入后续批次

---

## ✅ 本轮已修复（commit 待推）

| # | 严重性 | 问题 | 修复 |
|---|---|---|---|
| 1 | **高** | `read_file` 的 `start + limit` 可溢出（`offset` 大 + `limit` 大 → panic / 回绕） | `saturating_add`（tools/read_file.rs） |
| 2 | **高** | compaction 把摘要作为独立 User 消息前置，与首条 User 消息相邻 → Anthropic 等要求角色交替的 provider 直接 400 | 摘要**合并进首条 User 消息**（core/agent.rs） |
| 3 | **高** | Anthropic provider 忽略 `request.max_tokens`（summarize 的 2048 上限失效；thinking 时静默超预算） | `body()` 用 `request.max_tokens.unwrap_or(self.max_tokens)`（provider/anthropic.rs） |
| 4 | **中** | TUI busy 时提交先清空输入再拒绝 → 草稿丢失 | busy 检查提前到清空之前（tui/lib.rs submit） |
| 5 | **中** | `truncate` 只保留开头 → 编译错误在日志末尾被整体截掉 | 保留**头 2/3 + 尾 1/3**，中间标注丢弃（tools/util.rs） |
| 6 | **中** | periph_init 对 esp32 输出 STM32 HAL 骨架（USART1/HAL_UART_Init），与注入的 esp32 cheatsheet 自相矛盾 | 非 STM32 part（part 前缀判断，stm32h750 仍走 HAL）→ 通用骨架（tools/periph_init.rs） |
| 7 | **中** | decode_address 无符号 size 边界 → 符号间隙地址被归因成上一函数 `+0x大偏移` | 增加 `address >= addr+size` 覆盖检查（tools/decode.rs） |
| 8 | **低** | shell 危险命令检测漏 `%VAR%`（cmd 环境变量间接执行） | METAPROGRAMMING_PATTERNS 加 `%`（tools/shell.rs） |

---

## ⏳ 待修复（建议后续批次，按优先级）

### 高
- **TUI 锁粒度**：turn 全程持 agent 锁，命令循环阻塞，Esc 中断在长 turn 内失效（tui/lib.rs:206-208）→ 需小粒度锁 + Cancel 高优通道，**单独批次大改**
- **shell 拦截绕过**：`x=rm; $x -rf`（变量间接）与反斜杠续行 `r\<nl>m` 仍可绕过（one-shot 模式）→ 需命令解析 AST 级检测，误报风险需仔细权衡
- **web_fetch SSRF**：DNS rebinding 未防护（域名解析到内网/元数据地址）→ 需 resolver 校验 A/AAAA，**单独批次**

### 中
- spill 文件 24h 无条件清理，删除仍被旧 session 引用的文件（core/agent.rs:438-453）→ 引用计数或软链校验
- `.undo` 目录无限累积无轮转（core/journal.rs）→ 上限 + 轮转
- MaxIterations 回滚后 session 不保存（core/agent.rs:823-829）→ 回滚后 save
- kb 目录整体加入 allowed_roots，write_file 可污染内置知识库（core/agent.rs:562）→ 只读挂载或排除
- `merged_for` 静默忽略项目配置的 max_iterations/thinking/context_budget/auto_approve（core/config.rs）→ 补合并或显式警告
- `replace_session` 不重置 read_hashes/ledger_seq/elf_gate 等会话态（core/agent.rs:490）→ 切换时重置
- 子代理取消时 temp 目录泄漏（core/agent.rs:1089 / subagent.rs:131）→ 取消路径清理
- 工具路径依赖（tool_call_dependencies）用原始字符串比较，`./a.txt` vs `a.txt` 不归一 → 并发读旧/半写（core/agent.rs:1003）
- monitor `spawn_blocking` 不响应 cancel，turn 取消后串口仍占用至 timeout（tools/monitor.rs:206）→ 接 ctx.cancel
- ctags 同步执行阻塞 async runtime 且无超时（tools/symbols.rs:275）→ spawn_blocking + 超时
- todo 并发丢失更新 + 非原子写（tools/todo.rs:25）→ 锁 + 原子写
- TUI try_send 静默丢命令（/session /provider 等 4 处）→ 统一 send_cmd
- TUI 持锁 await list_models（网络慢阻塞 cmd 循环）→ 移出锁外
- CLI --max-output-tokens u32 截断回绕（无 clamp）→ 统一 clamp
- TUI 终端 raw mode/假屏无 RAII 恢复（panic 时残留）→ Drop guard
- TUI 工具卡片明文渲染参数（可能泄露 key/口令）→ 脱敏摘要
- install/update 二进制无完整性校验 → 支持 .sha256

### 低
- compaction 预算只计 messages，未计 system prompt + tool schemas → 预算偏移
- PlanModePermission 白名单不完整（verify/build/flash 未拦）→ 补全
- Ledger seq 用行数推导，空行/损坏行错乱 → 显式 seq
- summarize 失败静默降级无日志 → 记录降级原因
- read_file header 0-based 与正文 1-based 不一致（分页易差一行）→ 统一
- web_fetch 超 200KB 静默截断无提示 → 标注
- discover_su_files 不认 .gitignore，纳入陈旧/无关 .su → 过滤或提示
- probe_baud 空闲线（持续 0xFF）时所有候选误判失败 → 空线提示
- esp32 cheatsheet 注入与通用骨架并存时提示不明确
- Unix add_user_path 假成功、replace_file tmp 残留、PATH 去重不展开 %VAR%（install.rs）

---

## 结论

- **安全相关已堵住**：危险命令 4 种绕过形态剩 2 种（变量间接/续行）待 AST 级修复；SSRF 需 resolver（两项均明确风险可控、暂不阻断使用）
- **正确性关键修复已完成**：角色交替 400、输出截断丢错误、read_file 崩溃、esp32 错误骨架
- **建议节奏**：下一批优先「TUI 锁重构」与「spill 清理引用」两项高/中；SSRF resolver 与 shell AST 检测作为安全专项
