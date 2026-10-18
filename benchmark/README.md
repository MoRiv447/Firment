# Firment 嵌入式基准（2026-10）

一套 **14 题、硬件在环** 的评测：每题都必须编译、烧录、跑起来，并**在板子上量出结果**
（逻辑分析仪 / 串口 / 调试探针）。不是"看代码像不像"，是"波形对不对"。

本目录是评测器本体：题面、判分规则、夹具驱动、断言引擎，以及当次结果的报告。

## 目录

| 路径 | 是什么 |
|---|---|
| `specs/caseNN.toml` | 每题的时间盒、权重、步骤、断言与阈值 |
| `engine/` | 判分引擎（步骤执行、断言、静态规则、报告） |
| `hal/` | 硬件抽象：串口总线、夹具 GPIO、逻辑分析仪、探针 |
| `grader/` | 人工/AI 裁定的接口 |
| `cli.py` / `campaign.py` | 单题判分 / 批量跑一轮 |
| `FROZEN.md` + `frozen-manifest.json` | 判分口径的冻结记录（当前 `r7`，27 个文件） |
| `freeze_check.py` | 校验判分器有没有被改过 |
| `RESULTS.md` | 当次结果：逐题三轮、构建说明、边界声明 |
| `ai-rulings.md` | 机器判不了的点，逐条裁定与复核方法 |

## 台架

- **DUT**：NUCLEO-G431RB（STM32G431RB，板载 ST-Link/V3 → 虚拟串口）
- **夹具**：CH340 的 DTR/RTS 驱动按键线（两根交叉耦合，见 `hal/gpio.py` 的注释）
- **分析仪**：fx2lafw（sigrok-cli），4 通道接 PA8 / PA5 / PB8 / PB9
- **需要人**：case09 要万用表，case03/12/14 要人看一眼

端口一律按 VID/PID + 描述解析，不写死 COM 号；同型号板并存时在 `config.toml` 里点名。

## 跑一遍

```bash
cp config.example.toml config.toml     # 填 testset / judge / reports 三个路径
python cli.py caps                     # 端口 / 探针 / 分析仪 自检
python cli.py readiness                # 逐题就绪度
python -m campaign firm case01 case04 case05 case06 case07 case08 case10 case11
```

`campaign` 每题会：清空工作区 → 把题面交给被测 agent → 烧录 → 按 spec 施加激励 →
采集 → 判分 → 出报告。判分器版本与每条断言的实测值都写在报告里。

## 测了什么、没测什么

- **测了 8 题**（01/04/05/06/07/08/10/11，57/100 权重）。
- **没测 6 题**（02/03/09/12/13/14，43/100 权重）：
  - 12/14 需要相机对着板子，本机机位拍不到；
  - 13 需要蓝牙对端在线；
  - 03/09 需要人在跑分时回答一句（09 还要万用表读数）。
- 没测的题**不计入分母**，也不假装测过。

## 判分口径

`FROZEN.md` 记录了 `r1 → r7` 每一次改动、为什么改、影响面多大。当前 `r7`。
`python freeze_check.py` 会比对 27 个文件的 merkle，改过就会报警——包括我自己。

改判据的纪律：**先证伪，再改**。每一条放宽/收紧都要附一份真实失败的报告编号，
以及"这条改动会不会放过真缺陷"的说明。

## 已知的判据缺口

- `eval_int` 不含 CMSIS 头文件，任何用 `FLASH_BASE` 写出来的页号都会被判红（见 `ai-rulings.md`
  的 case10 裁定）。
- `case02.F3` 写死 `channel = "PA8"`，而题面允许任选一个有定时器通道的脚；选 PA6 之类的
  实现物理上测不到。这条还没改。
