//! Board profiles (plan §5, item 3): what is on the desk, in one file per board.
//!
//! A profile exists to answer the three questions a session otherwise asks the user every
//! time it touches hardware:
//!
//! * **What is it** — `part` is the MCU part number `periph_init` takes, and `board` is
//!   the pinmap key its conflict check uses. Both are the *existing* vocabulary; this
//!   module does not invent a second one.
//! * **How to talk to it** — `chip` is a probe-rs target (`stm32g431rb`, not
//!   `STM32G431RBT6`; the two differ and only one of them works) and `baud` is what the
//!   serial monitor should open with.
//! * **What fits on it** — `flash_kb` / `ram_kb`, which is the difference between
//!   suggesting a library and suggesting one that links.
//!
//! The files live in `docs/boards/` and are compiled in, so a shipped binary has the same
//! profiles this checkout does — and a test parses every one of them, which is what keeps
//! the data honest rather than merely present.

use std::collections::BTreeMap;
use std::path::Path;

use crate::config::Config;

/// Every profile in the repository, compiled in.
///
/// The list is written out rather than discovered at runtime: a build that reads `docs/`
/// would produce a binary whose behaviour depends on a directory that does not ship with
/// it.
const EMBEDDED: [(&str, &str); 6] = [
    ("esp32s3", include_str!("../../../docs/boards/esp32s3.toml")),
    (
        "nucleo-g031k8",
        include_str!("../../../docs/boards/nucleo-g031k8.toml"),
    ),
    (
        "nucleo-g431rb",
        include_str!("../../../docs/boards/nucleo-g431rb.toml"),
    ),
    (
        "nucleo-h723zg",
        include_str!("../../../docs/boards/nucleo-h723zg.toml"),
    ),
    (
        "stm32f103c8t6",
        include_str!("../../../docs/boards/stm32f103c8t6.toml"),
    ),
    (
        "stm32f407vet6",
        include_str!("../../../docs/boards/stm32f407vet6.toml"),
    ),
];

/// One board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardProfile {
    /// The file stem: what `firm board use` takes on the command line.
    pub name: String,
    /// What a human calls it.
    pub display: String,
    /// MCU part number, for `periph_init`.
    pub part: String,
    /// Pinmap key, for `periph_init`'s conflict check.
    pub board: String,
    /// probe-rs target name, for `flash`.
    pub chip: String,
    pub core: String,
    pub flash_kb: u32,
    pub ram_kb: u32,
    pub baud: u32,
    /// How the board is programmed (`st-link`, `usb-serial-jtag`, …).
    pub probe: String,
    /// The thing worth knowing before choosing this board. Not decoration: it is where a
    /// profile admits that its `chip` is not a probe-rs target, or that 8 KB of RAM is 8 KB.
    pub notes: String,
    /// Named pins (`led`, `uart_tx`, …). Free-form keys on purpose: a board has whichever
    /// signals it has, and a fixed schema would make half of them unrepresentable.
    pub pins: BTreeMap<String, String>,
}

impl BoardProfile {
    /// Parse one profile. The `name` comes from the file, not the contents: a profile that
    /// could rename itself would be a profile `firm board use` cannot find.
    pub fn parse(name: &str, text: &str) -> Result<Self, String> {
        let value: toml::Value = text
            .parse()
            .map_err(|e| format!("{name}: not valid TOML: {e}"))?;
        // `.get`, never `value[key]`: indexing a `toml::Value` **panics** on a missing
        // key, so a broken profile would take the process down instead of saying which
        // field is wrong. A test pins that.
        let get = |key: &str| -> Result<String, String> {
            value
                .get(key)
                .and_then(|field| field.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| format!("{name}: `{key}` is missing or not a string"))
        };
        let number = |key: &str| -> Result<u32, String> {
            value
                .get(key)
                .and_then(|field| field.as_integer())
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("{name}: `{key}` is missing or not a number"))
        };
        // `[pins]` is optional: a board profile is still useful with only its chip and its
        // memory, and a board whose pins nobody has transcribed should not be rejected.
        let mut pins = BTreeMap::new();
        for (key, pin) in value
            .get("pins")
            .and_then(|table| table.as_table())
            .into_iter()
            .flatten()
        {
            if let Some(pin) = pin.as_str() {
                pins.insert(key.clone(), pin.to_string());
            }
        }
        Ok(Self {
            name: name.to_string(),
            display: get("display")?,
            part: get("part")?,
            board: get("board")?,
            chip: get("chip")?,
            core: get("core")?,
            flash_kb: number("flash_kb")?,
            ram_kb: number("ram_kb")?,
            baud: number("baud")?,
            probe: get("probe")?,
            notes: get("notes")?,
            pins,
        })
    }

    /// One line for `firm board list`.
    pub fn summary(&self) -> String {
        format!(
            "{:<16} {:<26} {:<12} {:>5} KB flash {:>4} KB RAM",
            self.name, self.display, self.core, self.flash_kb, self.ram_kb
        )
    }

    /// The full description `firm board show` prints.
    pub fn detail(&self) -> String {
        let mut out = format!("{} — {}\n", self.name, self.display);
        out.push_str(&format!("  part    : {}\n", self.part));
        out.push_str(&format!("  board   : {}\n", self.board));
        out.push_str(&format!("  chip    : {} (probe-rs target)\n", self.chip));
        out.push_str(&format!("  core    : {}\n", self.core));
        out.push_str(&format!(
            "  memory  : {} KB flash, {} KB RAM\n",
            self.flash_kb, self.ram_kb
        ));
        out.push_str(&format!("  probe   : {}\n", self.probe));
        out.push_str(&format!("  baud    : {}\n", self.baud));
        if !self.pins.is_empty() {
            let pins: Vec<String> = self
                .pins
                .iter()
                .map(|(signal, pin)| format!("{signal}={pin}"))
                .collect();
            out.push_str(&format!("  pins    : {}\n", pins.join(" ")));
        }
        out.push_str(&format!("\n{}\n", self.notes));
        out.push_str(
            "\nThe chip name is a probe-rs target, not a part number: check it with \
             `probe-rs chip list | grep <name>` before flashing a board you did not profile.\n",
        );
        out
    }
}

/// Every embedded profile, in name order.
pub fn all() -> Vec<BoardProfile> {
    EMBEDDED
        .iter()
        .map(|(name, text)| {
            BoardProfile::parse(name, text)
                .unwrap_or_else(|e| panic!("embedded board profile {name} is broken: {e}"))
        })
        .collect()
}

/// Find a profile by name, part number, or chip — case-insensitively.
///
/// All three, because the user knows the board by whichever of them is in front of them
/// ("nucleo-g431rb" on the silkscreen, `STM32G431RBT6` in a schematic, `stm32g431rb` in
/// their last `probe-rs` command).
pub fn find(query: &str) -> Option<BoardProfile> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    let profiles = all();
    let matches = |profile: &BoardProfile, exact: bool| {
        [&profile.name, &profile.part, &profile.chip, &profile.board]
            .iter()
            .any(|candidate| {
                let candidate = candidate.to_ascii_lowercase();
                if exact {
                    candidate == needle
                } else {
                    candidate.contains(&needle)
                }
            })
    };

    if let Some(found) = profiles.iter().find(|p| matches(p, true)) {
        return Some(found.clone());
    }
    // A fragment is enough when it is unambiguous — people type `h723`, not
    // `nucleo-h723zg` — and ambiguous fragments are refused rather than guessed: a switch
    // to the wrong board is a wrong chip and a wrong linker script, and the caller's error
    // names every profile so the next try is easy.
    let candidates: Vec<&BoardProfile> = profiles.iter().filter(|p| matches(p, false)).collect();
    match candidates.as_slice() {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

/// Point a config at this board, returning what changed (for the message the user reads).
///
/// `standard_tools` — `default_chip` and `monitor_baud` — are what today's `flash` and
/// `monitor` already read, so the switch takes effect immediately rather than waiting for
/// a new field to be honoured everywhere. `[board] active` records *which* profile those
/// came from, so the next session can say where its defaults came from.
///
/// It deliberately does not touch `monitor_port`: a port is which USB hole the cable is in,
/// not a property of the board, and guessing one would be wrong on every machine but the
/// author's.
pub fn apply(profile: &BoardProfile, config: &mut Config) -> Vec<String> {
    let mut changes = Vec::new();
    config.board.active = Some(profile.name.clone());
    changes.push(format!("[board] active = {}", profile.name));

    if config.tools.default_chip.as_deref() != Some(profile.chip.as_str()) {
        changes.push(format!(
            "[tools] default_chip = {}   (was {})",
            profile.chip,
            config.tools.default_chip.as_deref().unwrap_or("not set")
        ));
        config.tools.default_chip = Some(profile.chip.clone());
    }
    if config.tools.monitor_baud != profile.baud {
        changes.push(format!(
            "[tools] monitor_baud = {}   (was {})",
            profile.baud, config.tools.monitor_baud
        ));
        config.tools.monitor_baud = profile.baud;
    }
    changes
}

/// Whether a path is a board profile file, for a caller walking `docs/boards`.
pub fn is_profile_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_profile_parses_and_names_a_real_thing() {
        // The test that keeps the data honest: a typo in a field, or a board file added
        // without adding it to `EMBEDDED`, fails here rather than in front of a user who
        // is holding the board.
        let profiles = all();
        assert_eq!(profiles.len(), EMBEDDED.len());
        for profile in &profiles {
            assert!(!profile.part.is_empty(), "{}", profile.name);
            assert!(!profile.board.is_empty(), "{}", profile.name);
            assert!(!profile.chip.is_empty(), "{}", profile.name);
            assert!(profile.flash_kb > 0, "{}", profile.name);
            assert!(profile.ram_kb > 0, "{}", profile.name);
            assert!(profile.baud >= 9600, "{}", profile.name);
            assert!(!profile.notes.is_empty(), "{}", profile.name);
        }
    }

    #[test]
    fn the_profiles_the_plan_asked_for_are_all_there() {
        // STM32F1/F4/G0/G4/H7 + ESP32-S3 (plan §5, item 3). Named explicitly so a profile
        // deleted by accident cannot quietly take a family with it.
        let names: Vec<String> = all().into_iter().map(|p| p.name).collect();
        for expected in [
            "stm32f103c8t6",
            "stm32f407vet6",
            "nucleo-g031k8",
            "nucleo-g431rb",
            "nucleo-h723zg",
            "esp32s3",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn a_profile_is_found_by_any_name_the_user_has_for_it() {
        // Silk screen, schematic, and last probe-rs command — all three are in front of
        // different people, and the one they type is the one they have.
        for query in [
            "nucleo-g431rb",
            "NUCLEO-G431RB",
            "stm32g431rb",
            "STM32G431RB",
        ] {
            let profile = find(query).unwrap_or_else(|| panic!("{query} should resolve"));
            assert_eq!(profile.name, "nucleo-g431rb");
        }
        assert!(find(query_that_is_not_a_board()).is_none());
    }

    #[test]
    fn a_fragment_resolves_when_it_is_unambiguous_and_is_refused_when_it_is_not() {
        // Nobody types `nucleo-h723zg`; they type `h723`. That has one answer, so it works.
        assert_eq!(
            find("h723").map(|p| p.name),
            Some("nucleo-h723zg".to_string())
        );

        // `stm32` is four boards: the caller's error lists them, and guessing here would
        // pick a chip and a linker script at random.
        assert!(find("stm32").is_none());
        assert!(find("").is_none());
    }

    fn query_that_is_not_a_board() -> &'static str {
        "arduino-uno"
    }

    #[test]
    fn switching_a_board_sets_what_the_tools_already_read() {
        let profile = find("nucleo-g431rb").unwrap();
        let mut config = Config::default_config();
        config.tools.default_chip = Some("stm32f103c8".to_string());
        config.tools.monitor_port = Some("COM7".to_string());
        config.tools.monitor_baud = 9600;

        let changes = apply(&profile, &mut config);

        assert_eq!(config.board.active.as_deref(), Some("nucleo-g431rb"));
        assert_eq!(config.tools.default_chip.as_deref(), Some("stm32g431rb"));
        assert_eq!(config.tools.monitor_baud, 115200);
        // A port is which hole the cable is in, not a property of the board.
        assert_eq!(config.tools.monitor_port.as_deref(), Some("COM7"));
        // Every change is reported, so the user sees what the switch did.
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(changes[0].contains("active = nucleo-g431rb"));
        assert!(changes[1].contains("was stm32f103c8"));
        assert!(changes[2].contains("was 9600"));
    }

    #[test]
    fn switching_to_the_board_already_in_use_reports_only_what_it_did() {
        // Nobody wants three "changed" lines for a no-op, and a report that says it
        // changed things it did not is the start of not trusting the report.
        let profile = find("nucleo-g431rb").unwrap();
        let mut config = Config::default_config();
        apply(&profile, &mut config);
        let again = apply(&profile, &mut config);
        assert_eq!(again.len(), 1, "{again:?}");
        assert!(again[0].contains("active"));
    }

    #[test]
    fn a_profile_missing_a_field_says_which_one() {
        let broken = "display = \"x\"\npart = \"x\"\n";
        let error = BoardProfile::parse("broken", broken).unwrap_err();
        assert!(error.contains("board") || error.contains("chip"), "{error}");
        assert!(error.starts_with("broken:"), "{error}");
    }
}
