# Changelog

## v0.4.0-beta.1 粘贴误发送修复（2026-08-08）

- **Ctrl+V 粘贴修复**：Windows 终端会把粘贴内容作为快速按键流注入（末尾 Enter 触发提交），新增粘贴爆发检测——35ms 内连续涌入的纯文本键识别为一次粘贴，期间 Enter 视为换行，静默后整体走折叠粘贴
- ASCII 首字符短暂 hold（30ms）避免单键闪烁；中文/IME 字符不 hold，通过 retro-capture 收回已插入前缀，避免多字残留
- 保护窗口：粘贴落地后 120ms 内的 Enter 视为换行，防止紧随粘贴的回车误发送
- 事件循环 ticker 从 100ms 缩短到 25ms，hold/缓冲到期的字符按时落地
- 新增 7 个粘贴爆发状态机单元测试（多行流、单字符+回车、慢速输入、中文收回、保护窗口、修改键透传、字符不丢失）

## v0.3.0-beta.7 知识库维护补丁（2026-08-08）

- 修正 esp32-gpio strapping 说明：strapping 引脚为 GPIO 0/2/4/5/12(MTDI)/15(MTDO)；GPIO4 选择 VDD_SDIO 电压、GPIO12(MTDI) 选择 flash 电压（此前误删 GPIO4 并将 VDD_SDIO 错标到 GPIO12）
- 统一知识库 `common_mistake` 为数组格式（消除索引标量 / 速查数组 / 带标签表三种混用），TOML 全量校验通过
- vendor-index.md 的「自动发现」说明经代码核验已实现（`load_vendor_index_hint` 注入提示词），恢复为原准确表述

## v0.3.0-beta.7（2026-08-08）

- **硬件知识库（seed）**：新增 STM32 F1（RM0008）、STM32 G0（RM0444）、ESP32-S3 三个 family 与 7 张原创速查表（USART/DMA、TIM/PWM、时钟树、GPIO/EXTI、LPUART、USB-Serial 等）
- **知识库自动发现**：项目含 `docs/vendor-index.toml` 时自动注入提示词，要求 agent 涉及硬件问题先查再答
- **知识库完整性测试**：校验索引/速查表 TOML 可解析、`quickref.cheatsheet` 链接有效、`meta.schema_version` 存在
- 修正 esp32-gpio strapping 引脚清单（0/2/4/5/12/15；GPIO6-11 为 SPI flash 脚）
- 移除过时 .example 模板，README/说明链接指向正式索引

## v0.3.0-beta.6（2026-08-08）

- README：补名字彩蛋——Firment 取自 *firmament*（苍穹），故意少一个 **a** 融合 firmware + agent（中英文同步）
- 配置模板与 README：标注 `compaction_strategy` 默认值与选项语义（`summarize` / `drop` / `off`）
- Pin 大文件预算保护：`/pin` 的文件超过上下文预算 30% 时在 TUI 显示警告，建议只固定关键源码文件
- README 安全说明：补充 `FIRMENT_VERSION` 固定版本安装的一行示例
- 仓库描述更新为：*General-purpose coding agent for firmware & embedded development, built in Rust.*

## v0.3.0-beta.5（2026-08-08）

- 新增 Pin 固定：`/pin <路径>` / `/unpin <路径>`，按会话持久化；上下文压缩时固定文件逐字回填全文
- 新增 `compaction_strategy`：`summarize`（默认，旧轮全摘要）/ `drop`（最近 3 轮逐字 + 中间 5 轮摘要 + 更早直接丢弃）/ `off`（禁用自动压缩）
- 符号索引升级：自动优先 universal-ctags（JSON 输出，60 秒缓存），未安装时回退内置正则；新增 `[tools] symbols_backend = auto | ctags | regex`
- README：修正 `context_budget_chars` 的配置层级（顶层而非 `[tools]`），补充新选项示例

## 更早版本（摘要）

- **v0.3.0-beta.1**：编辑可靠性栈——事务编辑/`/undo`、CAS 防并发覆盖、verify 工具、diff-first 批准、并行工具调用、上下文压缩、符号索引（正则）、失败分类
- **v0.3.0-beta.2**：工具输出外溢、参数 schema 校验、改动台账（`/ledger`）
- **v0.3.0-beta.3**：模型摘要压缩、缓存稳定前缀、重复读取去重、按 API 轮次驱逐
- **v0.3.0-beta.4**：verify 硬门（代码强制）、路径沙箱、CI（fmt/clippy/test 双平台）
- **v0.2.x**：危险命令安全闸加固、TUI 增强、GitHub Releases 一键安装、开源化（MIT/中英 README/BENCHMARK）
