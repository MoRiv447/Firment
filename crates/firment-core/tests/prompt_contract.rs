//! 提示词自己的契约测试。
//!
//! 为什么值得单独一个文件：以前没有任何一条测试守着提示词，所以它可以一直长 —— 每次给工具
//! 加参数就在这里补一句，而注册表改了它也不知道。分叉的两个方向都实测过（2026-10-17 基准）：
//! 写了没配置的 `verify` ⇒ 模型每道题撞一次 `[InvalidInput] verify tool is not configured`；
//! 沉默了已配置的 `la` ⇒ 仪器接好了也没人想到用。
//! 下面每条都是**先失败再修**的钉子：改内容之前先把这些断言翻成新的契约。

use firment_core::PromptTools;

fn tools_with(names: &[&str]) -> PromptTools {
    PromptTools::new(&names.iter().map(|n| n.to_string()).collect::<Vec<String>>())
}

/// 静态体里不许出现工具标识符：存在性与参数以 schema 为准，写在这里一定会和注册表分叉。
#[test]
fn static_contract_does_not_name_tools() {
    for name in [
        "hil",
        "verify_command",
        "elf_analyze",
        "periph_init",
        "pinmap",
        "la",
        "observe",
        "debug analyze",
    ] {
        assert!(
            !contains_word(&firment_core::context::static_body(), name),
            "static prompt names the tool `{name}` — that belongs in its schema description"
        );
    }
}

/// 整词匹配。裸 `contains("hil")` 会被 "while" 命中 —— 一条会假报的守卫比没有守卫更糟，
/// 因为它教会下一个人去放宽断言。
fn contains_word(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    let hb = h.as_bytes();
    let nb = n.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let mut from = 0;
    while let Some(offset) = h[from..].find(&n) {
        let start = from + offset;
        let end = start + nb.len();
        let left_ok = start == 0 || !is_word(hb[start - 1]);
        let right_ok = end >= hb.len() || !is_word(hb[end]);
        // 多词短语（"debug analyze"）内部空格不参与边界判断，只查两端。
        let inner_ok = nb.iter().all(|b| *b == b' ' || b.is_ascii_alphanumeric());
        if left_ok && right_ok && inner_ok {
            return true;
        }
        from = end;
    }
    false
}

/// 未配置的工具既不进注册表也不进提示词；配好了就必须出现（沉默也是错）。
#[test]
fn instrument_line_follows_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let with = firment_core::default_system_prompt(
        dir.path(),
        &tools_with(&["read_file", "la", "observe"]),
    );
    let without = firment_core::default_system_prompt(dir.path(), &tools_with(&["read_file"]));

    assert!(
        with.contains("logic analyzer"),
        "a configured instrument must be announced: {without}"
    );
    assert!(
        !without.contains("# Tools in this session"),
        "no capability tools => the whole section is omitted, not left empty"
    );
}

/// 空注册表（例如直接调用工具的单测里）不许让提示词长得像"什么都有"。
#[test]
fn pin_section_exists_only_when_the_tool_does() {
    let dir = tempfile::tempdir().unwrap();
    let with =
        firment_core::default_system_prompt(dir.path(), &tools_with(&["read_file", "pinmap"]));
    let without = firment_core::default_system_prompt(dir.path(), &tools_with(&["read_file"]));
    assert!(with.contains("# Pin registry"));
    assert!(
        !without.contains("# Pin registry"),
        "pinmap absent but the prompt still orders the model to use it"
    );
}

/// 两处知识库节必须有各自的名字 —— 同名会被读成"重复注入"，而它们内容不同。
#[test]
fn the_two_knowledge_sections_have_distinct_headings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vendor-index.toml"), "[my-board]\n").unwrap();
    let prompt = firment_core::default_system_prompt(dir.path(), &tools_with(&["read_file"]));
    assert!(prompt.contains("# Built-in hardware knowledge base"));
    assert!(prompt.contains("# Project hardware notes"));
    assert_eq!(
        firment_core::section_titles(&prompt)
            .into_iter()
            .filter(|t| t.contains("knowledge") || t.contains("hardware notes"))
            .count(),
        2
    );
}

/// 提示词里不许出现板级事实。**这是基准有效性的一部分**：考的就是"agent 能不能自己查到"，
/// 把"B1 高有效""OLED 在 PB8/PB9"写进提示词等于把选择题变成背诵题。
#[test]
fn prompt_carries_no_board_facts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vendor-index.toml"), "[placeholder]\n").unwrap();
    let prompt = firment_core::default_system_prompt(
        dir.path(),
        &tools_with(&["read_file", "pinmap", "la", "hil"]),
    );
    // 扫整份装配结果而不是只扫静态体：生成段（环境 / pin / 知识库指针）同样不许往里塞板级事实。
    // 本用例的项目 index 是占位内容，所以命中的只可能是我们自己的装配代码。
    let haystack = prompt.to_ascii_lowercase();
    for fact in [
        "active low",
        "active high",
        "PULLDOWN",
        "PULLUP",
        "IT_RISING",
        "0x3C",
        "0x78",
        "PB8",
        "PB9",
        "PC13",
        "nucleo-g431rb",
        "STM32G431",
    ] {
        assert!(
            !haystack.contains(&fact.to_ascii_lowercase()),
            "the assembled prompt asserts a board fact (`{fact}`) the agent is supposed to \
             look up itself"
        );
    }
}

/// 知识库里不许写"某道考题的答案"。产品资料可以讲封装、讲 AF 表、讲踩过的坑，
/// 但不许出现用例编号或以用例为对象的一句话 —— 那既污染评测，也把产品资料写窄了。
#[test]
fn shipped_knowledge_base_has_no_test_case_answers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = 0usize;
    let mut scan = |path: std::path::PathBuf| {
        if let Ok(text) = std::fs::read_to_string(&path) {
            checked += 1;
            for (n, line) in text.lines().enumerate() {
                let lower = line.to_ascii_lowercase();
                assert!(
                    !regex_lite_case_id(&lower),
                    "{}:{} reads like an answer written for one benchmark case: {line}",
                    path.display(),
                    n + 1
                );
                assert!(
                    !lower.contains("phantom-pin trap") && !lower.contains("case0"),
                    "{}:{} names a test case in product documentation: {line}",
                    path.display(),
                    n + 1
                );
            }
        }
    };
    let docs = root.join("docs");
    scan(docs.join("vendor-index.toml"));
    if let Ok(entries) = std::fs::read_dir(docs.join("cheatsheets")) {
        for entry in entries.flatten() {
            scan(entry.path());
        }
    }
    assert!(
        checked > 10,
        "expected to scan the shipped KB, saw {checked} files"
    );
}

/// 上一轮去答案化漏掉的形状：句子里没有用例编号，却把**被测的那块板**写进了"常见错误"。
/// 原句是 `Picking PE* pins on a G431RB (LQFP64) — no Port E on that package`，而
/// `shipped_knowledge_base_has_no_test_case_answers` 只按 `caseNN` 抓，所以放行了。
///
/// 判据按区块而不是按文件：封装引脚表、AF 表、board profile 里的板级事实是产品内容（`pinmap`
/// 就靠它们），保留；"坑 / 经验教训"这一类是叙述性提示，点名被测板等于把这道题的答案写进每个
/// 用户都会读到的资料里。
#[test]
fn pitfall_lists_do_not_name_the_board_under_test() {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    files.push(docs.join("vendor-index.toml"));
    if let Ok(entries) = std::fs::read_dir(docs.join("cheatsheets")) {
        files.extend(entries.flatten().map(|e| e.path()));
    }
    let mut entries_checked = 0usize;
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut in_mistakes = false;
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let trimmed = lower.trim_start();
            if trimmed.starts_with('[') {
                in_mistakes = false;
                continue;
            }
            if trimmed.starts_with("common_mistake") {
                in_mistakes = true;
                continue;
            }
            if !in_mistakes {
                continue;
            }
            if trimmed == "]" {
                in_mistakes = false;
                continue;
            }
            if !trimmed.starts_with('"') {
                continue;
            }
            entries_checked += 1;
            for dut in ["g431rb", "nucleo-g431", "lqfp64", "lqfp-64", "nucleo-g4"] {
                assert!(
                    !lower.contains(dut),
                    "{}:{} is a pitfall entry naming the board under test ({dut}): {line}",
                    path.display(),
                    n + 1
                );
            }
        }
    }
    assert!(
        entries_checked > 100,
        "expected to scan the pitfall lists, saw {entries_checked} entries — \
         the state machine stopped matching the array shape"
    );
}

/// `case01 lesson` / `(caseNN …trap)` 这类句式。
fn regex_lite_case_id(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    let mut i = 0;
    while let Some(pos) = find_from(bytes, b"case", i) {
        let mut j = pos + 4;
        let mut digits = 0;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            digits += 1;
            j += 1;
        }
        if digits >= 2 {
            return true;
        }
        i = pos + 4;
    }
    false
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if hay.len() < from + needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&k| &hay[k..k + needle.len()] == needle)
}
