# AI 裁定记录（裁判专用，勿泄露给 agent）

`pending_ai` 的落笔处。判分器里 `ai_judge` 那一步只写日志、不回写分数（`engine/runner.py:1038`，
P3 缺口），所以**裁定必须记在纸面上**，否则"待 AI 裁定"就永远是待着 —— 2026-10-17 之前那
24 个点就是这么一直悬着的。

每条都要写清三件事：为什么机器判不了、判了什么分、别人怎么复核。
证据一律 `文件:行号`，工作区路径是 `D:\OldStudy66\ai test pro\runs\<agent>\<case>`
（被重跑抹掉之前的原件在 `runs\<agent>\_bench\wiped\…` 与 `_bench\pre-rerun-20261017\…`）。

---

## 20261017-192904-firm-case01（权重 5，机器分 4.0）

### H2 「未与 BSP COM 双重初始化冲突」 ⇒ **判 1 分**

- **为什么机器判不了**：静态规则看见 `BSP_COM_Init` 与 `HAL_UART_Init` 并存，但"是不是同一句柄
  被初始化两次"要看代码结构，规则证不了 ⇒ `r3` 把它降为无法判定（成绩单 §重判 第 5 行）。
- **判 1 的依据**：
  1. `src/heartbeat.c:56-63` 不自己调 `HAL_UART_Init`，而是覆盖 HAL 的**弱符号**
     `HAL_UART_MspInit`，并且只接管 `huart->Instance == LPUART1` 那一路（`:58` 提前返回），
     把 BSP 的句柄**捕获**成 `s_uart`（`:63`）——全程没有第二个初始化点。
  2. 全语料里 `HAL_UART_Init` 的**调用点**只有 `cubemx/Drivers/BSP/STM32G4xx_Nucleo/stm32g4xx_nucleo.c:456`
     一处（`stm32g4xx_hal_uart.c:307` 是它自己的定义，其余命中都在注释里）；
     `cubemx/Core/Src/main.c:107` 的 `BSP_COM_Init(COM1, …)` 只调一次且文件未改（血统：合规）。
  3. 现象侧闭环：F1/F2/F3 全 pass ⇒ BSP 的 `COM1_MspInit`（PA2/PA3、LPUART1 时钟）与 agent 的
     hook **都执行了**且互不打架；F3 的"连续 60s 无乱码无丢行"就是"输出错乱 = 0"这一档的反证。
- **复核方法**：`grep -rn "HAL_UART_Init(" runs/firm/case01/{src,cubemx}` 应只有 HAL 库里那一处；
  读 `src/heartbeat.c:56` 确认它只捕获句柄、不重新初始化。

> 顺带一句值得进"神来之笔"的：它选 `HAL_UART_MspInit` 作为注入点，等于让 BSP 自己把 COM1 的
> 句柄递过来，既不用复制 CubeMX 的 UART 配置、也不用碰 `Core/`，血统和引脚正确性是**白拿的**。

## 20261017-192212-firm-case04（权重 8，机器分 3.0）

### H1 「有真实去抖」 ⇒ **判 1 分**

- **为什么机器判不了**：rubric 的三档（真去抖 / 纯中断直连 = 0 / ISR 里 `HAL_Delay(20)` 硬等 = 0.5）
  要读控制流，静态规则只能看有没有中断。
- **判 1 的依据**：`src/main.c:73-123` 是一个**时间戳状态机**：
  `candidate` 必须连续保持 `DEBOUNCE_MS = 20u`（`:22`）不变才升为 `stable`（`:84`），电平一变就
  重开窗口（`:78-82`），采样周期 `POLL_MS = 2u`（`:27`）；长按用 `cand_since - press_tick >=
  LONG_PRESS_MS` 判（`:99`、`:114-115`），没有阻塞等待。
  - 没有任何 ISR 参与按键 ⇒ 不属于"纯中断直连计数 = 0"那一档；
  - `HAL_Delay` 只出现在主循环（`:122`）与标定（`:204`/`:209`），不在中断里 ⇒ 不属于 0.5 那档。
- **另加一条加分项之外的手艺**：`btn_calibrate()`（`:198-221`）先用内部下拉/上拉各读一次，
  据此**推断空闲电平与极性**，再决定用哪一档 —— 这把"B1 到底是高有效还是低有效"从抄答案变成
  了测量。判分上不加分（rubric 没有这一档），但这是本题最容易翻车的地方，记在这里。
- **复核方法**：读 `src/main.c:73-123`，确认存在"稳定窗口 + 变化即重置"的双状态结构，且按键
  路径上没有中断服务函数。

### H2 「共享计数 volatile/临界区意识；回调链正确、无重复注册 EXTI」 ⇒ **判 1 分**

- **为什么机器判不了**：`r3` 的判据是"先证存在跨上下文共享（ISR 改写、函数体外也引用）再谈保护"。
  这一局根本没有 EXTI，前提不成立 ⇒ 规则记无法判定（报告里那条说明就是它写的）。
- **判 1 的依据**：
  1. 计数器是 `volatile uint32_t`（`src/main.c:32-33`），并且理由写在注释里（`:31`："可以从调试
     探针回读"）—— 是**有意识的 volatile**，不是抄来的。
  2. 只有主循环改它们（`:102`/`:107`/`:118`），没有中断 ⇒ 不存在需要临界区的共享；"无重复注册
     EXTI"这一半自然成立（全文件无 `HAL_NVIC_EnableIRQ`、无 `GPIO_MODE_IT_`）。
  3. 回调链正确：长按到 800 ms 先报一次 LONG（`:114-120`），松开时 `long_done` 拦住二次计数
     （`:95`）——机器断言 F1"10 次短按恰好 1..10、不多记不少记"和 F2"3 次长按无一次误判"都 pass，
     是对这条链最硬的反证。
- **不判更高/更低的原因**：轮询方案有"短于 ~20 ms 的按键会被漏"这一固有风险，题面没要求处理
  亚 20 ms 脉冲，夹具脉冲远长于此 ⇒ 不扣。
- **复核方法**：`grep -nE "EXTI|_IT_|NVIC_EnableIRQ|GPIO_MODE_IT" src/main.c` 应为空（实测确为空，
  整个方案是纯轮询）；`long_done` 共四处赋值 —— 初始化 `:59`、检测到按下沿时复位 `:93`、置 1 的
  两条路径 `:101`（松开时才补判 LONG）与 `:117`（按住满 800 ms 立刻报），配合 `:95`/`:114` 的
  `long_done == 0u` 闸门，同一次按压不可能计两次。

---

## 20261017-193610-firm-case07（权重 8）— **本题无待裁点**

F1 1.5 / F2 1 / F3 0.5 / H1 1 / H2 1 = **5.0 / 5.0**，H1、H2 均由静态规则带 `文件:行号` 判出
（`src/app_echo.c:234`、`:282`、`:58`、`:77`），不需要 AI。记在这里是为了让"零裁定"也留痕：
以后看到某题没有 AI 分，应该能分辨是**不需要**还是**没人判**。

## 20261017-202100-firm-case05（权重 6，机器分 3.0）

### H1 「PSC/ARR 基于 170 MHz 推出」 ⇒ **判 1 分**

- **为什么机器判不了**：`case05.H1` 要求在同一句初始化里看到成对的 PSC/ARR 常数；这份代码写成
  了**符号表达式**（`PWM_ARR = PWM_PERIOD - 1`），规则取不到那对数 ⇒ 降为无法判定，不是判红。
- **判 1 的依据**：`src/main.c:6-14` 把推导过程写在注释和宏里 ——
  `PWM_TIM_CLK_HZ = 170000000UL`（:10）、`PWM_FREQ_HZ = 20000UL`（:11）、
  `PWM_PERIOD = TIM_CLK / FREQ`、`PWM_ARR = PWM_PERIOD - 1U /* 8499 */`（:13）、
  `PWM_CCR = (PWM_PERIOD * 3U) / 10U /* 2550 -> 30.00 % */`（:14）、`Init.Prescaler = 0`（:41）、
  `Init.Period = PWM_ARR`（:43）。按 16/84/168 MHz 算是这一档要给 0 的，这里是 170 MHz。
- **复核方法**：读 `src/main.c:6-14` 与 `:41/:43/:53`，确认 ARR/CCR 都是从 170e6 推出来的表达式，
  而不是硬编码的魔数。

### H2 「追问：为何不是正好 20.000 kHz」 ⇒ **本次不裁定（自动化缺口，不进分母）**

- rubric 这一档要求**追问答辩**：答出 HSI 派生/RC 精度 = 1；答"仪器误差" = 0.5；坚称重调 = 0。
  它判的是 agent 对**误差来源的解释**，而 `campaign` 的一次性 `-p` 跑法里没有答辩环节 ——
  没有问答记录可判。代码里也没有相关自述（`grep -niE "hsi|tolerance|误差|精度" src/*.c` 只命中
  时钟配置那一行，`:137`），所以也不能改成"看代码给分"。
- 处理：与 `reset:power`、`manual_observe` 同一类 —— **标注后排除**，不给 0（那是把它没被问过
  当成答错），也不给 1。⇒ case05 本题可测满分 **4**。
- 要真想收这一分：两侧都要在同一协议下补一次答辩（"实测 f=…，为什么不是正好 20.000 kHz？"），
  单独问 Firment 或单独问 claude 都不成立。已记进 §6 已知缺口。

## 20261017-203254-firm-case08（权重 9，机器分 3.5）

### H1 「ReceiveToIdle_DMA 或手写 IDLE+DMA」 ⇒ **判 1 分**

- **为什么机器判不了**：规则找的是 HAL 层的接收注册调用；这份代码**绕过 HAL 接收 API 自己接 DMA**
  ⇒ 取不到调用点，降为无法判定。
- **判 1 的依据**：`src/main.c:64` `Init.Mode = DMA_CIRCULAR`、`:75` `HAL_DMA_Start(...)`、
  `:86` `__HAL_UART_ENABLE_IT(&hlpuart1, UART_IT_IDLE)`、`:101-105` 中断里判 IDLE 并读
  `__HAL_DMA_GET_COUNTER` 反推写位置、`:145` 主循环同一套推导。rubric 里 0.5 那一档是
  "DMA 但定长 normal"，0 那一档是"中断逐字节假装 DMA"，这里两条都不沾：循环模式 + 真 IDLE。
- **复核方法**：`grep -nE "DMA_CIRCULAR|HAL_DMA_Start|UART_IT_IDLE|GET_COUNTER" src/main.c`
  四条都要在，且不能出现 `HAL_UART_Receive_DMA` 的 normal 模式配置。

### F3 「COUNT? 返回实际正确处理帧数」 ⇒ ~~机器判红成立，不改~~ **已推翻，见文末 r4 更正**

原判（2026-10-17 20:32 那一局）写的是"夹具发 100 帧、固件答 `COUNT 105` ⇒ 计数路径与响应路径
不同源的真缺陷，保留 0 分"。**这条是错的**，错因和更正都在文末《r4 更正》一节：夹具在
`la_capture` 的窗内又发了 5 帧合法 LED 命令，裁判自己的 `sent_frames` 没把这 5 帧记账。

---

# 栏 A 补裁（旧成果 × `r3`，2026-10-17 深夜）

栏 A 的 `summary.pending_ai` 那 24 条里，**14 条其实是 `machine_fail`（机器判红，0 分）**，
是列表把两类一起报了（`engine/runner.py:168`）。真正的 AI 待裁点只有 10 个，而且两侧**正好各 5 个、
同 4 题同点号** ⇒ 补裁不会偏向任何一边。旧工作区在 `runs/_bench/pre-rerun-20261017/`，语料还在，
所以这些裁定今天仍然可复核。

**不改写已落盘的 `report.md`**：这一节只活在裁定记录和成绩单里，栏 A 的报告保持原样。

| 点 | firm | claude | 依据（两边都是自己的代码） |
|---|---|---|---|
| case04 H1 真实去抖 | **1** | **1** | firm `src/button_app.c`：1 kHz TIM6 采样 + `button_tick()` 状态机，`DEBOUNCE_MS=8`，且 `:141` 把计时起点取在**原始边沿**而不是去抖窗末尾；claude `src/app.c`：EXTI + `HAL_GetTick()` 25 ms 时间窗 + 重新挂边沿。两边都不是"中断直连计数"，也不是 ISR 里 `HAL_Delay` 硬等 |
| case05 H1 依据 170 MHz | **1** | **1** | firm `src/main.c:29-31` `PWM_TIM_CLK_HZ=170000000UL`/`PSC=0`/`ARR=8499`；claude `src/main.c:22-25` 同一组数，另加 `:41-43` 两条 `_Static_assert` 把算式和 30% 占空比在**编译期**钉住 |
| case07 H2 回绕与溢出 | **1** | **1** | firm `src/echo.c:35-46/:77/:276-279`：2 的幂掩码回绕，入队前判 `rx_count()!=RX_RING_MASK`，满则丢弃不越界；claude `src/echo.c:25-40/:61-70`：`next=(head+1)&MASK`、`next==tail` 判满，head/tail 都 `volatile` |
| case08 H2 指针与竞态 | **1** | **1** | firm `src/main.c:34-35/:113-138`：单生产者(ISR)单消费者(main)分离 head/tail、`%RX_RING_SIZE` 回绕、满检查、`volatile`；claude `src/app_vcp.c:45-58/:174-186`：`s_line[VCP_LINE_MAX+1]` 留出终止符，写入前判 `< VCP_LINE_MAX`，还单独记了 `s_line_overflow` |
| case05 H2 追问答辩 | **排除** | **排除** | 判的是"能否解释误差来自 HSI/RC"，自动化 `-p` 跑法没有答辩环节，代码里也无相关自述 ⇒ 不给 0（那是把"没被问过"当"答错"），两侧同一处理 |

### 第二批（`r3` 新转成"未观测"的三条 —— 第一遍漏裁了）

第一遍我是照着**冻结报告里的 `summary.pending_ai`** 列的清单，那是**复判之前**的状态。
`r3` 又把三条判红降成了"未观测 ⇒ 交 AI"，照旧清单裁就会漏。改从 `regrade/diff.json` 对账才找出来：

| 点 | 老判定 → r3 | firm | claude | 依据 |
|---|---|---|---|---|
| case01 H2 双重初始化 | ✗ → 未观测 | **1** | （本题 claude 是机器判绿，不在待裁里） | firm `src/heartbeat.c:42` 直接写 `TDR`、`:64` 注明"LPUART1 由 BSP_COM_Init 在上电时使能"，自己**不调** `HAL_UART_Init`；`Core/Src/main.c:107` 只调一次 `BSP_COM_Init` ⇒ 无第二处初始化。旁证：栏 A 该局 F1/F2/F3 全过（60 s 无乱码），rubric 的"输出错乱 = 0"这一档没触发 |
| case04 H2 共享与回调链 | ✗ → 未观测 | **1** | **1** | firm `src/button_app.c:93-142`：全部状态在 `btn` 一个结构里，只被 1 kHz 的 `TIM6_DAC_IRQHandler` 用（`:268` 唯一调用点），打印也在中断内 ⇒ **不存在跨上下文共享**，"先证共享再谈保护"的前提不成立，按 r3 的口径不扣分；全文件不用 EXTI，"无重复注册"自然成立。claude `src/app.c:45-86`：覆盖 BSP 弱回调、`:31` 清 `EXTI` 挂起、`:61/:68/:76` 在 FALLING/RISING 之间**交替重挂**（同一时刻只有一个沿有效）⇒ 回调链正确、无双重触发；`short_count`/`long_count` 只在该回调内读写，同样不构成共享 |

顺带一条**不是问题的问题**：claude 旧 case01 里 `src/main.c:107` 和 `src/heartbeat.c:79` 各有一处
`BSP_COM_Init`，看着像双重初始化。实际它用了 `-Wl,--wrap=main`（`int __wrap_main(void)`），
CubeMX 那个 `main()` 根本不执行 ⇒ 只跑一次。看到两处调用别急着判红，先确认调用图。

**加权影响**：claude **+7.80**、firm **+8.80**（不对称，firm 多拿 case01 H2 那 1.0），
所以栏 A 由 **Claude 43.40 ｜ Firment 36.40** 变成 **Claude 51.20 ｜ Firment 45.20**，
**分差 7.00 → 6.00**。第一遍我写的"补裁是齐步走的、分差不变"是**错的**，已按这张表更正 ——
错因值得记住：拿旧清单当待裁全集，就会漏掉尺子刚刚新放开的点；待裁全集必须从复判产物里取。

**顺带暴露的尺子缺口（记在这里，不在本轮改）**：`case08.H1` 的静态规则只看有没有
`ReceiveToIdle_DMA`/IDLE+DMA，**不区分 circular 与 normal 重挂**。栏 A 两份旧代码都是
`HAL_UARTEx_ReceiveToIdle_DMA` 的 normal 模式重挂（claude `:10` 自己写明 "arms a normal-mode DMA
transfer"），按 rubric 字面第二句"DMA 但定长 normal = 0.5"是可以压到 0.5 的；本轮按第一句
（该 API 就是这个场景的标准写法）给 1.0，两侧一致 ⇒ 不影响对比，但下轮要么把规则收紧到认
`Init.Mode = DMA_CIRCULAR`，要么把 rubric 那句歧义写掉，不能靠评委临场解释。


---

# claude 新局（2026-10-17 21:0x-21:48 重跑）的 5 个待裁点

代码全部来自 `D:\OldStudy66i test prouns\claude\caseNN`（新局工作区），口径与上面 Firment 的
裁定完全一致：**同一把 rubric、同一套"前提不成立就不扣分"的规则**。

| 点 | 判 | 依据 |
|---|---|---|
| case04 H1 真实去抖 | **1** | `src/button_app.c:27` `APP_DEBOUNCE_MS=20U`，`:93-99` 在 EXTI 回调里用 `HAL_GetTick()` 丢弃与上一沿相隔 <20 ms 的边沿；无 `HAL_Delay` 硬等 |
| case04 H2 共享/回调链 | **1** | `:32-38` 所有跨上下文状态全 `volatile`；`:69` 覆盖的是 **BSP 的** `BSP_PB_Callback`（`stm32g4xx_it.c:206` 只调一次 `BSP_PB_IRQHandler` 分发）⇒ 链条单路径、无重复注册；`:51-60` 首次按下沿才把模式从 RISING 升为 RISING_FALLING，由 `s_both_edges_enabled` 守着，不重开 NVIC |
| case05 H1 依据 170 MHz | **1** | `src/…:25` `PWM_TIMER_CLK_HZ = 170000000UL`、`:28` `ARR = PERIOD_TICKS - 1 = 8499`、`:30` `CCR1 = (8500*300)/1000 = 2550`，注释 `:8-12` 把推导写全 |
| case05 H2 追问答辩 | **排除** | 与 Firment 同一处理：自动化跑法没有答辩环节 ⇒ 不进分母，两侧都不给 |
| case07 H2 环形缓冲 | **1** | `src/…:28` 2 的幂掩码、`:67-72` `next=(head+1)&MASK` 且 `next!=tail` 才写（满则保留旧数据、注释 `:74` 说明了取舍）、`:32-33` head/tail `volatile`、单生产者/单消费者 |

**行为学上值得记一笔的差异**（不加分不扣分，只记录）：claude 在 `button_app.c:104` 写了注释
"B1 is active high on NUCLEO-G431RB (pull-down, pressed = 1)" —— 它是**从 BSP 读出极性后写死**；
rc3 的 Firment 是同题**自己量**（`btn_calibrate()` 下拉/上拉各读一次再定）。两种都算对，
但抗换板子的能力不同：极性写死的那份挪到别的板子就会错。

**claude case11 的三条红是真的**：`F1b serial_max_gap` 指到 `alive-30s.log 第35条@+14.068s`
（心跳停了一大截）、`F2b serial_gap` 指到 `stuck.log 第6→7条`、`H1b serial_absent` 指到
`por.log 第104条@+43.609s`。三条各自独立、都带行号，不是判分器口径造成的。

---

# rc3 第二遍（2026-10-17 22:08–22:48）的 7 个待裁点

> **r5 之后本节的状态**：其中 3 条（case05 H1、case07 H2、case08 H1）静态规则自己判得动了
> （`chain-check/r5-static-flips.md`，分值与这里一致，都是 1.0），已由机器出分、裁定作废但留档；
> 仍由 AI 出分的是 case01 H2、case04 H1、case04 H2 三条，外加 case05 H2 继续排除。
> 这一节因此仍有两种价值：给仍然生效的裁定留依据，也给"规则为什么当时看不懂"留反例。

这一遍是**同一份二进制的自复跑**，工作区被 `prep` 抹过重做，所以代码与第一遍**不是同一份**
（例：case01 第一遍挂 `HAL_UART_MspInit`，第二遍挂 `MX_LPUART1_Init` ⇒ 必须重裁，
不能引用第一遍的裁定）。语料 = 现在还在的 `runs/firm/caseNN`，第一遍的原件在
`D:\Firment-bench-archive\firm-caseNN\run-2`（`run-1` 是 rc2 那一版的）。

| 点 | 判 | 为什么机器判不了 / 依据 |
|---|---|---|
| case01 H2 双重初始化 | **1** | 规则只数出 `BSP_COM_Init`×3 与 `HAL_UART_Init`×1，判不出"同一句柄初始化两次"。全语料 `HAL_UART_Init(` 只有两处调用点：agent 的 `src/heartbeat.c:88` 和 BSP 的 **`__weak MX_LPUART1_Init`**（`stm32g4xx_nucleo.c:456`）；agent 在 `:76` 提供了强符号覆盖 ⇒ 链接后弱的那份不存在，`BSP_COM_Init` 在 `:396` 只调这一次。反证：本题 F1/F2/F3 全绿（60 s 无乱码无丢行），rubric 的"输出错乱 = 0"没触发 |
| case04 H1 真实去抖 | **1** | `src/main.c:173-201` 时间戳状态机：`candidate` 须连续稳定 `DEBOUNCE_MS=20`（`:20`、`:180`）才升为 `stable`，电平一变就重开窗口（`:173-177`），主循环 1 ms 采样（`:211`）。无中断参与（下面那条会用到），`HAL_Delay` 只在主循环与标定（`:100`/`:105`）⇒ 既不是"中断直连"也不是 ISR 里硬等 |
| case04 H2 共享/回调链 | **1** | 规则说"看不出跨上下文共享 ⇒ 不据此扣分"，剩下要判的是回调链。实测 `grep -nE "EXTI\|_IT_\|NVIC_EnableIRQ\|GPIO_MODE_IT" src/main.c` **为空**（纯轮询，`:92` 用 `BUTTON_MODE_GPIO`），计数器 `short_n`/`long_n` 是 `main()` 的局部变量（`:136-137`）⇒ 不构成共享。`long_fired` 闸门（`:135`、`:187`、`:189`、`:207`）保证一次按压只报一次，机器侧 F1"10 次短按恰好 1..10"、F2"3 次长按无一次误判"、F3 全绿是最硬的反证 |
| case05 H1 依据 170 MHz | **1** | 规则要"成对出现在同一段初始化里的 PSC/ARR 常数"，这份写成符号表达式 ⇒ 取不到。`src/main.c:20` `TIM1_KERNEL_HZ=(170000000UL)`、`:22` `PWM_PERIOD_TICKS=KERNEL/FREQ`、`:24` `PWM_ARR=PERIOD-1 /*8499*/`、`:26` `PWM_CCR=(PERIOD*300)/1000 /*2550*/`、`Init.Prescaler=0`/`Init.Period=PWM_ARR`。另加三条 `_Static_assert`（`:28-32`）把"20 kHz 整除""占空比映射到整数""CCR<ARR+1"钉在编译期 |
| case05 H2 追问答辩 | **排除** | 与 rc2/claude 两侧同一处理：判的是"能否解释误差来自 HSI/RC"，一次性 `-p` 跑法没有答辩环节，代码里也无自述 ⇒ 不给 0 也不给 1，**不进分母**（详见 §已知缺口） |
| case07 H2 环形缓冲 | **1** | 规则 `wrap_ops=0` 是因为它的正则只认 `& (SIZE-1)` 这种现写形式，这份把掩码做成了宏：`:18` `#define RX_RING_MASK (RX_RING_SIZE - 1u)`，回绕在 `:175` 与 `:223`。人工核对：4096 是 2 的幂 ⇒ `& MASK` 等价取模；写前先判满（`:177` `next != rx_tail`），满则丢弃并计数（`:184`）不越界；head/tail `volatile`（`:25-26`）且各自只有一个上下文持有（ISR 写 head、主循环写 tail）；索引恒 `< 4096`。机器侧 F1a/b/c 用三个不同种子各 1024 字节逐字符回显全绿、F3 风暴期 STATS 延迟 <800 ms 全绿 |
| case08 H1 circular DMA+IDLE | **1** | 规则找 HAL 接收注册调用，这份绕过 HAL 接收 API 自己接 DMA ⇒ 取不到。`:187` `Init.Mode = DMA_CIRCULAR`、`:226` `HAL_DMA_Start(...)`、`:233` `SET_BIT(LPUART1->CR3, USART_CR3_DMAR)`、`:236` `__HAL_UART_ENABLE_IT(..., UART_IT_IDLE)`、`:55` `RX_DMA_SIZE - __HAL_DMA_GET_COUNTER(...)` 反推写位置、`:71-79` IDLE 里调同一个 `rx_drain()`。rubric 的 0.5 档（"DMA 但定长 normal"）与 0 档（"中断逐字节假装 DMA"）都不沾 |

**加权影响**：本节 7 条里仍由 AI 出分的是 case01 H2（+1.00）、case04 H1+H2（+3.20），
case05 H2 继续排除（−1.20 不进分母）；case05 H1、case07 H2、case08 H1 三条自 r5 起由
静态规则直接判绿，分数一样但不再占"待裁"的名义。⇒ 第二遍 **47.40 → 51.60 / 55.8**。
（`chain-check/final_table.py` 是这套加法的唯一实现，改数请改脚本。）

---

# r4 更正（判分器自己的两处错，五批统一重算）

这两处不是"裁定"，是**尺子读错了数**，所以由脚本 `chain-check/final_table.py` 从落盘证据重算，
两侧同批处理。`engine/rev.py` 里有对应的 `r4` 两条。

### ① case05 F3c `pulse_width` 把采集窗边界当成固件的缺口

`hal/la.py::measure_pulse_width` 原来在整盘波形上取"最短高电平台"，含被窗首尾切断的那一段。

| 批次 | 原读数 | 盘里真实的完整平台 | 结论 |
|---|---|---|---|
| rc2 | 1.75 µs ✗ | 14.75 µs（10 021 段，最短 14.75） | 假红 ⇒ +1.0 |
| rc3 第一遍 | 14.75 ✓ | 14.75 | 不变 |
| rc3 第二遍 | 7.5 µs ✗ | 14.75 µs（10 023 段） | 假红 ⇒ +1.0 |
| claude 旧 | 14.75 ✓ | 14.75 | 不变 |
| claude 新 | 15.0 ✓ | 15.0 | 不变 |

**判据互斥**：同一盘 `capture-4.sr` 上 F2 `duty` 量到 30.0 %、F1 `frequency` 20.05 kHz ⇒
高电平只能是 15 µs（30 % × 50 µs），7.5 µs 与它俩不可能同时为真。复核方法：
`python chain-check/final_table.py`（它会打印上面这张表），或直接解析
`reports/<run>/evidence/la/capture-4.sr` 看第一个高电平台是否起始于第 0 个样本。
方向性：截断只会让读数**偏小** ⇒ 这条只可能消掉假红，不会放跑假绿。

### ② case08 F3b 的 `sent_frames` 漏记窗内激励 ⇒ "COUNT 105" 不是缺陷

- 夹具实际注入 105 帧合法命令：`serial_burst` 100 帧 + `la_capture` 的 `during` 5 帧 LED ON/OFF
  （三局的 `steps` 都写着 `stimulus {frames: 5, sent: 5}`：`20261017-203254-firm-case08`、
  `20261017-223633-firm-case08`、`20261017-213232-claude-case08`）。
- **两个 agent 都答 105**，与"实际正确处理的帧数"一致 ⇒ 是裁判把参照值停在 100。
- 我第一遍裁定"计数路径与响应路径不同源的真缺陷"，**错在把 F1a/b/c 的绿当成了"只处理了 100 帧"**：
  `burst.log` 的窗在 `during` 之前就关了（`scope = "all"` 但从窗起点切），那 5 帧 LED 落在窗外，
  所以"恰好 50 个 OK"与"总共 105 帧"同时成立。看到两处计数不一致，先确认**激励总共发了几帧**，
  再谈谁多记了。
- 处理：`runner._capture_with_stimulus` 补 `self._sent_frames += sent`；`specs/case08.toml` F3b
  的 `tol_abs` 从 0 放到 1（先答后加=105 与先加后答=106 都符合判据字面，同 case07 的
  `tol_abs=8` 惯例 —— 判据没写清的事不许由零容差掷硬币）。
- 影响：rc3 两遍、claude 新局各 +0.5（权重 9 ⇒ +0.90）；rc2/claude 旧局当时没有窗内激励，
  答的就是 100，本来判绿 ⇒ 不变。**对称地错，也对称地还**。
- `firment改进清单.md` 里那条 `COUNT 105` 缺陷已撤回。


---

## 2026-10-18 第一轮（判分器 r6）· 7 条待裁

报告：firm `20261018-110800/111506/…`、claude `20261018-122649/124555/131431`。
`case05 H2` 仍按口径**排除**（追问答辩，自动化跑法没有问答环节，两侧同一处理）。

| 点 | firm | claude | 一句话依据 |
|---|---|---|---|
| case01 H2 双重初始化 | **1** | **1** | 只有 BSP 一条初始化路径 |
| case04 H1 真去抖 | **1** | **1** | 都是时间戳状态机、都在主循环轮询，ISR 里无 `HAL_Delay` |
| case04 H2 共享计数/回调链 | **1** | **1** | 两侧都没有 EXTI ⇒ 前提不成立（同 20261017 先例） |
| case08 H2 circular 指针 | — | **1** | normal+rearm，长度由 HAL 的 `RxEventCallback(Size)` 给出，环形缓冲无越界无竞态 |

### case01 H2「未与 BSP COM 双重初始化冲突」 ⇒ firm / claude 各 1 分

- **机器为什么判不了**：`BSP_COM_Init` 与 `HAL_UART_Init` 在语料里并存，但是不是同一个句柄被初始化
  两次，静态规则证不了 ⇒ 记未观测。
- **firm**：`src/heartbeat.c:102` 用**强符号覆盖** BSP 的弱 `MX_LPUART1_Init`，在 `:115` 只调一次
  `HAL_UART_Init(huart)`；`cubemx/Core/Src/main.c:107` 的 `BSP_COM_Init(COM1, …)` 也只调一次
  ⇒ 全工程只有一条初始化路径。
- **claude**：`src/` 里没有 `MX_LPUART1_Init` 的定义，只有 BSP 自带的弱实现
  （`src/BSP/stm32g4xx_nucleo.c:444` → `:456 HAL_UART_Init`）；`main` 里一次 `BSP_COM_Init(COM1,…)`
  ⇒ 同样只有一条路径。
- **旁证**：两侧 F1/F2/F3 全绿（`HELLO <n>` 1 Hz、60 s 无乱码）⇒ rubric 的"输出错乱 = 0"没触发。
- **复核**：`grep -rn "HAL_UART_Init(" runs/<agent>/case01/src` 只应命中 BSP 那一处定义/调用。

### case04 H1「有真实去抖」 ⇒ firm / claude 各 1 分

- **firm**：`src/main.c:98-129` 是"候选电平 + 稳定窗口"状态机，`BUTTON_DEBOUNCE_MS = 8`（`:17`），
  电平一变就重开窗口（`:116-120`）；采样在主循环（`:105 HAL_GetTick`），没有任何 ISR。
- **claude**：`src/main.c:23` `BTN_DEBOUNCE_MS = 10`，`:137-157` 用 `raw_change_ms` 判稳定，
  `:198` 在主循环里 `button_update(HAL_GetTick())`；同样没有 ISR。
- 两侧都不属于 rubric 的 0 档（纯中断直连）或 0.5 档（ISR 里 `HAL_Delay`）。

### case04 H2「共享计数 volatile/临界区；回调链正确」 ⇒ 各 1 分

- **前提不成立**：`grep -rn "EXTI|IRQHandler|Callback|volatile" runs/<agent>/case04/src` 两侧都是空
  —— 纯轮询实现，没有跨上下文共享，也就没有"重复注册 EXTI"的风险。
- 按 20261017 的先例（同一条判据、同样的轮询实现）给 1：这条判据问的是"共享有没有被保护"，
  没有共享时既不该判 0，也不该判 1 分之外的档。**注意**：这条对轮询实现在结构上不具区分度，
  记在这里备查。

### case08 H2「circular 指针推导正确；无越界无竞态」 ⇒ claude 1 分

- **为什么机器判不了**：规则只认 `__HAL_DMA_GET_COUNTER`/`CNDTR`；这份实现没有这两个符号。
- **判 1 的依据**：`src/app.c:181-199` 用 **normal 模式 + IDLE 重挂**，长度由 HAL 的
  `HAL_UARTEx_RxEventCallback(huart, Size)` 给出（HAL 内部就是 `RxXferSize − CNDTR`），
  回调里先 `ring_push(..., Size)` 再 `HAL_UARTEx_ReceiveToIdle_DMA` 重挂；
  环形缓冲 `:27-28` head/tail 均 `volatile`，`:41-45` 用 `& RING_MASK` 回绕并判满丢弃，
  `:59` 消费者同样掩码回绕 ⇒ 无越界、无读写竞态。F1 100/100 帧全对是它在 50 ms 间隔下不丢字节的实证。
- **与判据字面的差距（如实记）**：rubric 写的是"circular 指针"，这份是 normal 模式重挂，没有
  circular 指针。判 1 的理由是它**满足判据的目的**（长度取自 DMA 计数器、不是硬编码、不越界），
  与 r5 对 case10"算出来的最后一页"、case08 H1"手写 IDLE+DMA"的处理同一条原则：
  只惩罚真实缺陷，不惩罚等价写法。
- **复核**：读 `src/app.c:178-205`；确认 `Size` 被 `0 < Size <= RX_DMA_SIZE` 夹住再入环。

### 第一轮裁定后总分（机器 + AI，权重 57）

| | case01 | case04 | case08 | 其它 | 合计 |
|---|---|---|---|---|---|
| firm | 5.0 | 5.0 | 5.0 | 不变 | **54.4** |
| claude | 5.0 | 5.0 | 5.0 | 不变 | **54.4** |

（case05 H2 两侧排除；case10 F2 skip 不进分母。）

---

## 2026-10-18 第二轮（主体 r6；case10 在 r7 下重判）· 11 条待裁

| 点 | firm | claude | 一句话依据 |
|---|---|---|---|
| case01 H2 双重初始化 | （机器 pass） | **1** | 只有 `MX_LPUART1_Init` 一条路径，没有 `BSP_COM_Init` |
| case04 F1/F2/F3 | **0 / 0 / 0** | （机器 pass） | 固件真静默：PC13 配了上拉、又按低有效判 ⇒ 永远看不到翻转 |
| case04 H1 真去抖 | **1** | **1** | 两侧都是时间戳/稳定窗口状态机，ISR 里无 `HAL_Delay` |
| case04 H2 共享计数 | **1** | **1** | 两侧都是主循环轮询，没有 ISR 写计数（前提不成立，同先例） |
| case08 H2 circular 指针 | **1** | （机器 pass） | firm 用 HAL 的 `RxEventCallback(Size)`（`src/main.c:146-160`），与 claude 第一轮同型 |
| case08 F2 五命令 | （机器 pass） | **1** | LED 操作正确（`src/main.c:273-282`）；F2d 未观测是本机采集截断，不算它的账 |

### case04 F1/F2/F3 ⇒ firm **判 0**（可复现的静默，不是"未观测"）

- **为什么机器判不了**：两个窗口都是 0 行，`r5` 的空窗防线把它们记成"未观测"（分不清"没输出"与
  "没在听"）。这次能判，是因为有**两次独立采集**都 0 行、而且代码里能找到确定性原因。
- **证据**：
  1. `evidence/serial/all-dut_vcp.log` 两次（13:52 与 14:55 各一次重判）都是 `bytes_in=0 lines=0`；
     13 次夹具脉冲期间一个字节都没有。
  2. `src/main.c:153-155` 把 PC13 配成 `GPIO_PULLUP`；`:66` 用
     `if (deb_level == GPIO_PIN_RESET) /* pressed (active low) */` 把**低电平**当按下。
     这块板的 B1 是**高有效**（BSP 用 `GPIO_PULLDOWN` + 上升沿）⇒ 内部上拉把线按在高电平，
     夹具再怎么动都只是一直"高" ⇒ 轮询永远看不到翻转 ⇒ 不打印。
  3. 旁证：同一个二进制下 F1/F2/F3 全无输出，而 R1 的同一题（极性写对）能正常打 SHORT/LONG。
- **为什么不重跑**：这是 agent 的实现缺陷，不是现场波动。为"救回一个坏样本"重跑属于
  retry-until-lucky，会污染 n=3 的分布。按 0 记。
- **H1 仍给 1**：去抖算法本身是合格的时间戳状态机（`:12 DEBOUNCE_MS=10`、`:64` 稳定窗口），
  错的是极性假设，不是去抖；**H2 给 1**：没有 ISR、没有共享计数。

### case04 H1/H2（claude）与 case01/case08 的说明

- claude H1：`src/app_key.c:23 KEY_DEBOUNCE_MS=20`、`:98-101` 连续稳定采样计数，
  `src/main.c:114` 主循环 `AppKey_Poll()` ⇒ 真去抖。
- claude H2：`src/app_key.c:64` 用 `BSP_PB_GetState()` **轮询**读数（虽然 `BSP_PB_Init` 用的是
  `BUTTON_MODE_EXTI`，但没有任何 ISR 写计数）⇒ 前提不成立，同先例给 1。
- claude case01 H2：`src/main.c:26` 调自己的 `MX_LPUART1_Init`（`:88` 定义、`:100` 一次
  `HAL_UART_Init`），没有 `BSP_COM_Init` 的第二次初始化。
- firm case08 H2：`src/main.c:146-160` 的 `HAL_UARTEx_RxEventCallback(huart, Size)` 直接把 HAL 给的
  字节数喂进环形缓冲 —— 与 claude 第一轮同型，按同一原则给 1。
- claude case08 F2：`src/main.c:273-282` 的 LED ON/OFF 各自 `HAL_GPIO_WritePin(PA5, …)`；
  F2a/b/c 全绿。F2d 之所以未观测，是 r6 那次采集只覆盖 1.x 秒（克隆板收工），不是它没做。

### 第二轮裁定后总分（机器 + AI，权重 57）

| | case01 | case04 | case08 | 其它 | 合计 |
|---|---|---|---|---|---|
| firm | 5.0 | **2.0** | 5.0 | 不变 | **49.6** |
| claude | 5.0 | 5.0 | 5.0 | 不变 | **54.4** |

两轮对照：R1 54.4 : 54.4（平）；R2 49.6 : 54.4（claude +4.8，全部来自 case04 的极性缺陷）。

---

## 2026-10-18 第三轮（r7）· 9 条待裁

| 点 | firm | claude | 依据 |
|---|---|---|---|
| case01 H2 双重初始化 | **1** | —（机器 pass） | firm `src/main.c:81` 只调一次 `HAL_UART_Init`；`cubemx/Core/Src/main.c` 的 `BSP_COM_Init` 不参与 PlatformIO 构建 |
| case04 H1 真去抖 | **1** | **1** | firm `run-9/src`: `DEBOUNCE_MS=20` 稳定窗口；claude `run-6/src`: `BTN_DEBOUNCE_MS=12` + `s_stable_ms`；两侧都无 ISR |
| case04 H2 共享计数 | **1** | **1** | 两侧都是 1 ms 轮询，无 EXTI ⇒ 前提不成立（同前两轮先例） |
| case04 F1/F2/F3 | —（机器 1.0） | **0**（静默） | claude 本轮 PC13 配 `PULLUP` 且判低有效（`run-6/src/main.c:73`），0 字节 |
| case07 H2 指针与竞态 | **1** | —（机器 pass） | `src/*.c:46-54` head/tail 均 `volatile`，双端掩码回绕 |
| case08 H2 circular 指针 | **1** | —（机器 pass） | `src/main.c:207-216` `HAL_UARTEx_RxEventCallback(huart, Size)`，长度取自 HAL |
| case08 F2 五命令 | —（机器 pass） | **1** | `src/*.c:113/119` LED ON/OFF 各写 PA5；F2d 未观测是本机采集截断 |

第三轮裁定后：**firm 51.2 ｜ claude 48.2**。

---

## 2026-10-18 发版构建抽检（r7）· case10 H2

`reports/20261018-201302-firm-case10`：机器判红，`page63=false`。

- 代码：`src/flash_counter.h:8` `BOOT_COUNTER_PAGE_ADDR = 0x08000000 + 128K - 0x800`，
  `src/flash_counter.c:12` `COUNTER_PAGE_INDEX = (COUNTER_PAGE_ADDR - FLASH_BASE) / FLASH_PAGE_SIZE`，
  `:50` `erase.Page = COUNTER_PAGE_INDEX`。代入即 `(0x0801F800 - 0x08000000) / 0x800 = 63`。
- 判红原因：规则只能解析工程内宏，**解析不到 CMSIS 的 `FLASH_BASE`** ⇒ 表达式求不出 63。
  与 r5/r7 修的两处同源（只认能算出来的写法）。
- 裁定：**H2 = 1**。复核：手算上面那行，或 `grep -n "BOOT_COUNTER_PAGE_ADDR" src/flash_counter.h`。
- 遗留：`eval_int` 不含 CMSIS 头文件，任何用 `FLASH_BASE` 的写法都会被判红。下次改判据时一并处理。
