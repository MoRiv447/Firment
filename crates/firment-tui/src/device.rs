//! What this session is aimed at, and what it can measure with.
//!
//! The right column's DEVICE block, fed from the config. It is deliberately not
//! a device *status*: probe and port are discovered when the flash tool runs, so
//! at rest there is nothing truthful to show for them, and a panel that invents
//! a device would be worse than a panel that shows only what was configured.
//!
//! This is most of the answer to "what is this session set up to do", which is
//! otherwise spread across `config.toml`, the `flash` tool's defaults and the
//! logic analyzer's driver options.

use firment_core::{Config, LaConfig};

/// The configured target and build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Device {
    /// `[tools] default_chip` -- what `flash` uses when the tool passes none.
    pub(crate) chip: Option<String>,
    /// `[tools] build_command` -- what the `build` tool actually runs.
    pub(crate) build: Option<String>,
    /// `[la]`, when the logic analyzer is configured at all.
    pub(crate) la: Option<La>,
}

/// The logic analyzer's configuration, which is not the same thing as a capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct La {
    pub(crate) driver: String,
    pub(crate) rate: Option<String>,
    pub(crate) channels: Option<String>,
}

impl Device {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            chip: config.tools.default_chip.clone(),
            build: config.tools.build_command.clone(),
            // An unconfigured analyzer is `None` rather than an empty driver:
            // the section is then hidden entirely, which is the documented rule
            // for "nothing is set up yet" (docs/design/tokens.md).
            la: config.tools.la.as_ref().map(La::from_config),
        }
    }

    /// The DEVICE block's rows, empty when nothing is configured.
    ///
    /// `build` is here rather than in a BUILD panel because it is part of the
    /// same answer: the chip says what the binary is for, the command says how
    /// it is produced. A probe and a port are absent on purpose -- see the
    /// module note.
    pub(crate) fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some(chip) = &self.chip {
            rows.push(("chip", chip.clone()));
        }
        if let Some(build) = &self.build {
            rows.push(("build", build.clone()));
        }
        rows
    }
}

impl La {
    fn from_config(config: &LaConfig) -> Self {
        // An empty driver is the struct's own default, so it is treated as
        // "not set" rather than shown as a blank line.
        let driver = config.driver.trim();
        Self {
            driver: if driver.is_empty() {
                "(default)".to_string()
            } else {
                driver.to_string()
            },
            rate: config.samplerate.clone(),
            channels: config.channels.clone(),
        }
    }

    /// The LA block's rows: what a capture *would* use.
    pub(crate) fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![("driver", self.driver.clone())];
        if let Some(rate) = &self.rate {
            rows.push(("rate", rate.clone()));
        }
        if let Some(channels) = &self.channels {
            rows.push(("channels", channels.clone()));
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_device_has_no_rows_and_is_not_drawn() {
        let device = Device::default();
        assert!(device.rows().is_empty());
    }

    #[test]
    fn a_configured_chip_and_build_are_both_shown() {
        let device = Device {
            chip: Some("stm32f407vetx".to_string()),
            build: Some("cargo build --release".to_string()),
            la: None,
        };
        assert_eq!(
            device.rows(),
            vec![
                ("chip", "stm32f407vetx".to_string()),
                ("build", "cargo build --release".to_string()),
            ]
        );
    }

    #[test]
    fn the_device_block_never_invents_a_probe_or_a_port() {
        // They are discovered when the flash tool runs. A panel that showed a
        // guessed port would send someone to the wrong one.
        let device = Device {
            chip: Some("stm32f407vetx".to_string()),
            build: None,
            la: None,
        };
        let labels: Vec<&str> = device.rows().iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec!["chip"]);
    }

    #[test]
    fn the_analyzer_block_names_what_a_capture_would_use() {
        let la = La {
            driver: "fx2lafw".to_string(),
            rate: Some("8m".to_string()),
            channels: Some("0,1".to_string()),
        };
        assert_eq!(
            la.rows(),
            vec![
                ("driver", "fx2lafw".to_string()),
                ("rate", "8m".to_string()),
                ("channels", "0,1".to_string()),
            ]
        );
    }

    #[test]
    fn an_unset_driver_and_rate_do_not_become_blank_rows() {
        let la = La {
            driver: "(default)".to_string(),
            rate: None,
            channels: None,
        };
        // One row, not three with two blanks: a blank value reads as a bug.
        assert_eq!(la.rows(), vec![("driver", "(default)".to_string())]);
    }
}
