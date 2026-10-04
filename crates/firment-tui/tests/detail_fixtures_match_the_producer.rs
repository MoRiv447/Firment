//! A hand-built test event may not carry a field the real producer never fills.
//!
//! `AgentEvent::ToolEnd::detail` is filled for exactly the tools
//! `firment_core::agent::is_diff_tool` names. That is a deliberate channel-size contract, not
//! an oversight -- see the doc on the function. For a whole audit round the TUI had a branch
//! that read `detail` for `la`, three tests that hand-built an `la` event *with* a `detail`, and
//! a panel that therefore could not appear in a real session while its tests said it could. The
//! fixtures were the proof and the fiction at the same time: nothing in a test run can tell an
//! event the agent emitted from an event the test author typed.
//!
//! So the fixtures are checked against the producer. A literal that puts a `detail` on a tool
//! the producer never ships one for is refused here, whether it is an event handed to
//! `on_agent` or an `Item::Tool` pushed straight onto the transcript.
//!
//! The scan is textual, and its limits are worth stating: it brace-counts, so a string in this
//! file with an unbalanced brace inside would stretch a window, and a fixture whose tool name is
//! computed (`name: name.to_string()`) cannot be resolved at all -- that one is reported rather
//! than skipped, because "unverifiable" and "wrong" need the same fix here. The two floors below
//! are what make a silently-broken scan fail instead of passing on nothing.

use firment_core::agent::is_diff_tool;

/// The openings of every literal this rule applies to.
const LITERALS: [&str; 2] = ["AgentEvent::ToolEnd {", "Item::Tool {"];

/// The byte span of the struct literal opened at `open_brace`, brace-counted.
fn span(source: &str, open_brace: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut at = open_brace;
    while at < bytes.len() {
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1 - open_brace;
                }
            }
            _ => {}
        }
        at += 1;
    }
    panic!("a struct literal opened at byte {open_brace} never closes");
}

/// `name: "edit_file"` inside a literal, if it names one tool statically.
fn named(literal: &str) -> Option<String> {
    let (_, after) = literal.split_once("name:")?;
    let rest = after.trim_start().strip_prefix('"')?;
    let name = rest.split('"').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

fn carries_detail(literal: &str) -> bool {
    literal.contains("detail: Some(")
}

#[test]
fn no_hand_built_fixture_invents_a_detail_the_producer_never_ships() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs");
    let source = std::fs::read_to_string(path).expect("the TUI's own source is readable");

    let mut windows = 0;
    let mut with_detail = 0;
    let mut offenders: Vec<String> = Vec::new();

    for needle in LITERALS {
        let mut from = 0;
        while let Some(found) = source[from..].find(needle) {
            let start = from + found;
            let open = start + needle.len() - 1;
            let literal = &source[start..open + span(&source, open)];
            from = open + 1;
            windows += 1;
            if !carries_detail(literal) {
                continue;
            }
            with_detail += 1;
            let line = 1 + source[..start].matches('\n').count();
            match named(literal) {
                None => offenders.push(format!(
                    "{path}:{line}: a literal carries `detail: Some(..)` but its `name:` is not \
                     a string literal, so this rule cannot check it"
                )),
                Some(name) if !is_diff_tool(&name) => offenders.push(format!(
                    "{path}:{line}: `detail: Some(..)` on `{name}`, which the producer never \
                     ships a detail for -- whatever renders it cannot be reached by a real run"
                )),
                Some(_) => {}
            }
        }
    }

    // The floors. If the literal style or the scan changes, the rule must not quietly become a
    // loop over nothing.
    assert!(
        windows >= 20,
        "expected the TUI's fixtures to be scanned; found {windows} literals"
    );
    assert!(
        with_detail >= 2,
        "expected at least a couple of detail-carrying fixtures; found {with_detail}"
    );
    assert!(offenders.is_empty(), "\n{}", offenders.join("\n"));
}

#[test]
fn the_producer_still_ships_detail_for_the_two_tools_it_promised() {
    // The other half of the pair: if the set ever narrows, the fixtures above stay green while
    // the diff band stops rendering, and that is a different bug with the same symptom.
    assert!(is_diff_tool("edit_file"));
    assert!(is_diff_tool("write_file"));
    assert!(!is_diff_tool("la"));
    assert!(!is_diff_tool("shell"));
    assert!(!is_diff_tool("read"));
}
