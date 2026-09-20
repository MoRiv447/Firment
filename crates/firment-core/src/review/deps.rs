//! Dependency / supply-chain review (plan §4-D).
//!
//! Two inputs, both already available locally, and neither of them a network call:
//!
//! * `cargo metadata --offline` — the resolved graph, with each package's declared
//!   licence. This is the compliance half.
//! * `cargo audit --json` — the advisories half, when the tool is installed.
//!
//! The functions here are pure over those JSON strings, so the two rules that matter
//! are testable without either tool being present: what counts as a copyleft licence in
//! an expression like `MIT OR GPL-3.0-only`, and what happens when the advisory database
//! is simply not there (a note, never a finding — §16.4).

use serde_json::Value;

use super::{Finding, ReviewReport, Severity};

/// A licence family that has obligations beyond attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copyleft {
    /// GPL, AGPL, SSPL: obligations that reach the whole product in the usual firmware
    /// arrangement (one binary, no dynamic linking of the offender).
    Strong,
    /// LGPL, MPL, EPL: file-level or linking-level obligations. Real, but a different
    /// decision from strong copyleft, and a reviewer who is told "GPL" when it is "MPL"
    /// will spend the afternoon on the wrong problem.
    Weak,
}

impl Copyleft {
    fn severity(&self) -> Severity {
        match self {
            Copyleft::Strong => Severity::High,
            Copyleft::Weak => Severity::Medium,
        }
    }
}

/// The copyleft obligation in one SPDX licence expression, if any.
///
/// Expressions are small grammars, and the two operators mean opposite things for a
/// compliance question:
///
/// * `A OR B` is a **choice**, so the user may take the permissive branch — a
///   `MIT OR GPL-3.0-only` dependency is not a copyleft problem.
/// * `A AND B` is a **conjunction**, so every branch applies — `MIT AND GPL-3.0-only`
///   is.
///
/// Anything unrecognised is reported as no obligation: inventing one from an id nobody
/// can parse would put a red finding in front of a human who then has to guess.
pub fn copyleft_in(expression: &str) -> Option<Copyleft> {
    let expr = expression.trim();
    if expr.is_empty() {
        return None;
    }
    // Strip the outer parentheses of a wrapped sub-expression before splitting, or
    // `(MIT OR Apache-2.0)` would split into `(MIT` / `Apache-2.0)`.
    if expr.starts_with('(') && expr.ends_with(')') && balanced(&expr[1..expr.len() - 1]) {
        return copyleft_in(&expr[1..expr.len() - 1]);
    }
    if let Some(parts) = split_top_level(expr, " OR ") {
        // A choice: copyleft only when there is no permissive branch to take. One
        // permissive branch is enough, and that is the branch a consumer would take.
        let mut strictest: Option<Copyleft> = None;
        for part in parts {
            // `?` on the option: a permissive branch anywhere makes the whole choice
            // satisfiable without the obligation, which is the answer.
            let found = copyleft_in(part)?;
            strictest = Some(match strictest {
                Some(current) => current.max_severity(found),
                None => found,
            });
        }
        return strictest;
    }
    if let Some(parts) = split_top_level(expr, " AND ") {
        // A conjunction: every branch applies, so the strictest one decides.
        return parts
            .iter()
            .filter_map(|part| copyleft_in(part))
            .max_by_key(|c| match c {
                Copyleft::Weak => 0,
                Copyleft::Strong => 1,
            });
    }
    classify_id(expr)
}

impl Copyleft {
    fn max_severity(self, other: Copyleft) -> Copyleft {
        match (self, other) {
            (Copyleft::Strong, _) | (_, Copyleft::Strong) => Copyleft::Strong,
            _ => Copyleft::Weak,
        }
    }
}

/// One licence id, without the expression grammar around it.
fn classify_id(id: &str) -> Option<Copyleft> {
    let id = id.trim().trim_end_matches('+').to_ascii_uppercase();
    if id.is_empty() {
        return None;
    }
    if id.starts_with("AGPL") || id.starts_with("SSPL") || id.starts_with("GPL") {
        return Some(Copyleft::Strong);
    }
    // `LGPL` is checked after `GPL`, but neither prefix matches the other: the checks
    // are `starts_with`, so `LGPL-3.0` needs its own arm.
    if id.starts_with("LGPL") || id.starts_with("MPL") || id.starts_with("EPL") {
        return Some(Copyleft::Weak);
    }
    None
}

/// Split on a separator that appears outside parentheses.
fn split_top_level<'a>(expr: &'a str, separator: &str) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = expr.as_bytes();
    let sep = separator.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && bytes[i..].starts_with(sep) {
            parts.push(&expr[start..i]);
            i += sep.len();
            start = i;
            continue;
        }
        i += 1;
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(&expr[start..]);
        Some(parts)
    }
}

/// True when every parenthesis in this fragment is matched.
fn balanced(expr: &str) -> bool {
    let mut depth = 0i32;
    for c in expr.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// One resolved package, as much as a licence review needs.
struct Package {
    name: String,
    version: String,
    license: Option<String>,
    /// A package that declares a licence *file* and no SPDX id.
    license_file: bool,
}

/// A third-party package: one with a `source` (the workspace's own crates have none).
fn dependencies(metadata: &Value) -> Vec<Package> {
    metadata["packages"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|p| !p["source"].is_null())
                .map(package_of)
                .collect()
        })
        .unwrap_or_default()
}

fn package_of(package: &Value) -> Package {
    Package {
        name: package["name"].as_str().unwrap_or("?").to_string(),
        version: package["version"].as_str().unwrap_or("?").to_string(),
        license: package["license"]
            .as_str()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string()),
        license_file: package["license_file"].as_str().is_some(),
    }
}

/// The dependency report: the licence distribution, plus a finding for every dependency
/// whose licence puts an obligation on the product.
///
/// The workspace's own crates are skipped — they have no `source`, and listing the
/// user's own licences as "dependencies" would both pad the distribution and hide the
/// real count.
pub fn review_metadata(metadata_json: &str) -> Result<ReviewReport, String> {
    let metadata: Value =
        serde_json::from_str(metadata_json).map_err(|e| format!("cargo metadata: {e}"))?;

    let deps = dependencies(&metadata);
    if deps.is_empty() {
        return Err("cargo metadata listed no third-party packages".to_string());
    }

    let mut report = ReviewReport::new("dependencies");
    report.detail(format!("{} third-party packages", deps.len()));

    let mut distribution: std::collections::BTreeMap<String, usize> = Default::default();
    for dep in &deps {
        *distribution
            .entry(
                dep.license
                    .clone()
                    .unwrap_or_else(|| "not declared".to_string()),
            )
            .or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, usize)> = distribution.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let listed = sorted
        .iter()
        .map(|(license, count)| format!("{license} ×{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    report.detail(format!("licences: {listed}"));

    for dep in &deps {
        let Some(license) = &dep.license else {
            // No SPDX id. Two different situations, and the fix differs, so they are
            // two different findings rather than one vague one.
            let mut finding = Finding::new(
                format!("dep-license-missing-{}-{}", dep.name, dep.version),
                format!("licence not declared: {} {}", dep.name, dep.version),
                Severity::Medium,
                "dependency",
                if dep.license_file {
                    "The crate declares a licence file but no SPDX identifier, so no tool can answer 'is this permissive?' without a human opening it."
                } else {
                    "The crate declares neither an SPDX identifier nor a licence file."
                },
            )
            .with_fix("Open the crate's manifest (or its source) and record what it is; if it stays undeclared, treat it as a compliance risk rather than an oversight.");
            finding.tags.push(
                if dep.license_file {
                    "license-file"
                } else {
                    "no-license"
                }
                .to_string(),
            );
            report.push(finding);
            continue;
        };
        if let Some(copyleft) = copyleft_in(license) {
            let (word, severity) = match copyleft {
                Copyleft::Strong => ("strong", copyleft.severity()),
                Copyleft::Weak => ("weak", copyleft.severity()),
            };
            report.push(
                Finding::new(
                    format!("dep-copyleft-{}-{}", dep.name, dep.version),
                    format!("{word} copyleft dependency: {} {}", dep.name, dep.version),
                    severity,
                    "dependency",
                    format!("Declared licence: {license}"),
                )
                .with_impact(match copyleft {
                    Copyleft::Strong => {
                        "A firmware image that links this statically inherits obligations that usually include publishing the whole source."
                    }
                    Copyleft::Weak => {
                        "Obligations are limited to this crate's own files (or to relinking), but they are obligations: check what the licence asks for before shipping the binary."
                    }
                })
                .with_file("Cargo.lock")
                .with_fix("Confirm the licence is acceptable for this product, replace the crate, or isolate it behind a process/linkage boundary that the licence allows.")
                .with_tags(&["license", "copyleft"]),
            );
        }
    }
    Ok(report)
}

/// What a cached manifest says about one package's licence.
pub struct LicenseFacts {
    pub license: Option<String>,
    pub license_file: bool,
}

/// The licence review that does not need the resolved graph.
///
/// `cargo metadata --offline` needs **every** dependency in the local cache; a lockfile
/// that mentions one crate nobody downloaded — a dev-dependency of a dependency, say —
/// makes it fail outright, and with no network there is no second attempt. Measured on
/// this workspace: `failed to download arbitrary v1.4.2 ... --offline was specified`.
///
/// `Cargo.lock` is complete by definition (it is what the build resolves against), so
/// the *list* is never in doubt. What is in doubt is the licence, which lives in the
/// manifest: `license_of` is handed each `(name, version)` and answers from wherever the
/// caller can find it. A package it cannot answer for is reported as
/// [`LicenseFacts::license`] `None` **and counted in a note**, because a review that
/// silently drops the crates it could not read is a review that reports "clean".
pub fn review_lockfile(
    lock_text: &str,
    license_of: &mut dyn FnMut(&str, &str) -> Option<LicenseFacts>,
) -> Result<ReviewReport, String> {
    let lock: toml::Value = toml::from_str(lock_text).map_err(|e| format!("Cargo.lock: {e}"))?;
    let entries = lock["package"]
        .as_array()
        .ok_or_else(|| "Cargo.lock has no [[package]] entries".to_string())?;

    let mut report = ReviewReport::new("dependencies");
    let mut unread: Vec<String> = Vec::new();
    let mut distribution: std::collections::BTreeMap<String, usize> = Default::default();
    let mut findings: Vec<Finding> = Vec::new();
    let mut third_party = 0usize;

    for entry in entries {
        // A path dependency carries no `source` at all: this workspace's own crate,
        // not a dependency to licence-review.
        if entry.get("source").and_then(|s| s.as_str()).is_none() {
            continue;
        }
        let name = entry["name"].as_str().unwrap_or("?");
        let version = entry["version"].as_str().unwrap_or("?");
        third_party += 1;

        let facts = license_of(name, version);
        let license = facts.as_ref().and_then(|f| f.license.clone());
        if facts.is_none() {
            unread.push(format!("{name} {version}"));
        }
        *distribution
            .entry(license.clone().unwrap_or_else(|| "not read".to_string()))
            .or_insert(0) += 1;

        // A manifest that is present but declares nothing is one finding per crate: a
        // human has to decide what that licence is. A manifest that is *absent* is not —
        // the action is one `cargo fetch` — so those are collected and reported as a
        // single finding below. Seventeen copies of "download it once" would bury the
        // findings that need a decision.
        let Some(facts) = facts else { continue };
        if license.is_none() {
            findings.push(
                Finding::new(
                    format!("dep-license-unknown-{name}-{version}"),
                    format!("licence not established: {name} {version}"),
                    Severity::Medium,
                    "dependency",
                    if facts.license_file {
                        "The cached manifest declares a licence file and no SPDX identifier."
                    } else {
                        "The cached manifest declares neither an SPDX identifier nor a licence file."
                    },
                )
                .with_fix("Read the crate's manifest and record what it is.")
                .with_tags(&["license"]),
            );
        }
    }

    if !unread.is_empty() {
        findings.push(
            Finding::new(
                "dep-license-unread",
                format!(
                    "{} dependencies were not read, so their licences are unknown",
                    unread.len()
                ),
                Severity::Medium,
                "dependency",
                format!(
                    "Cargo.lock lists them, but their manifests are not in the local cache: {}",
                    unread.join(", ")
                ),
            )
            .with_impact(
                "An unknown licence is not a permissive one: a copyleft crate nobody read \
                 is a copyleft crate nobody can clear for a firmware image.",
            )
            .with_fix(
                "Fetch them once with a network available (a build that uses them, or \
                 `cargo fetch`) and run this again.",
            )
            .with_tags(&["license", "unread"]),
        );
    }

    if third_party == 0 {
        return Err("Cargo.lock lists no third-party packages".to_string());
    }

    // Copyleft is decided from whatever licences *were* read: the same classifier the
    // metadata path uses, so the two paths cannot disagree about a licence.
    for entry in entries {
        // A path dependency carries no `source` at all: this workspace's own crate,
        // not a dependency to licence-review.
        if entry.get("source").and_then(|s| s.as_str()).is_none() {
            continue;
        }
        let name = entry["name"].as_str().unwrap_or("?");
        let version = entry["version"].as_str().unwrap_or("?");
        let Some(license) = license_of(name, version).and_then(|f| f.license) else {
            continue;
        };
        if let Some(copyleft) = copyleft_in(&license) {
            let word = match copyleft {
                Copyleft::Strong => "strong",
                Copyleft::Weak => "weak",
            };
            findings.push(
                Finding::new(
                    format!("dep-copyleft-{name}-{version}"),
                    format!("{word} copyleft dependency: {name} {version}"),
                    copyleft.severity(),
                    "dependency",
                    format!("Declared licence: {license}"),
                )
                .with_file("Cargo.lock")
                .with_fix(
                    "Confirm the licence is acceptable for this product, replace the crate, \
                     or isolate it behind a boundary the licence allows.",
                )
                .with_tags(&["license", "copyleft"]),
            );
        }
    }

    report.detail(format!("{third_party} third-party packages"));
    let mut sorted: Vec<(String, usize)> = distribution.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.detail(format!(
        "licences: {}",
        sorted
            .iter()
            .map(|(license, count)| format!("{license} x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if !unread.is_empty() {
        report.note(format!(
            "{} of {third_party} packages are not in the local cache, so their licences \
             were not read ({}…)",
            unread.len(),
            unread
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for finding in findings {
        report.push(finding);
    }
    Ok(report)
}

/// Answer a crate's licence from the local cargo registry cache.
///
/// The cache is what `cargo build` has already downloaded, so anything that has ever
/// been built here is answerable — which is exactly the set that matters when the graph
/// cannot be resolved. Returns `None` for a crate that is not there.
pub fn cached_license_of(name: &str, version: &str) -> Option<LicenseFacts> {
    let home = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            // Windows is not `$HOME`-shaped, and this project's CI runs on it.
            std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".cargo"))
        })
        .or_else(|| std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".cargo")))?;
    let root = home.join("registry").join("src");
    cached_license_in(&root, name, version)
}

/// `cached_license_of` with the cache root passed in, so a test can point it at a
/// fixture instead of the machine's real cache.
pub fn cached_license_in(
    root: &std::path::Path,
    name: &str,
    version: &str,
) -> Option<LicenseFacts> {
    let registries = std::fs::read_dir(root).ok()?;
    for registry in registries.flatten() {
        let manifest = registry
            .path()
            .join(format!("{name}-{version}"))
            .join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            continue;
        };
        let package = &value["package"];
        return Some(LicenseFacts {
            license: package["license"]
                .as_str()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string()),
            license_file: package.get("license-file").is_some(),
        });
    }
    None
}

/// What the advisory database behind a `cargo audit` run actually was.
///
/// The offline review's §4.5, and the reason it matters: an advisory database is a **live
/// feed**, so "0 advisories" from a snapshot taken two months ago reads *exactly* like "0
/// advisories from today's". The licence half of this report already distinguishes "not read"
/// from "read and clean"; this is the same honesty for the half that depends on a feed.
///
/// `AGENTS.md` records the incident that motivated it: on this machine
/// `~/.cargo/advisory-db` is a snapshot from 2026-08-11, and a report that did not say so was
/// a report that quietly answered a different question than the reader asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryDatabase {
    /// The raw `last-updated` string, which is what cargo-audit said — kept verbatim because
    /// a report should quote its source rather than re-render it.
    pub last_updated: String,
    /// How many advisories the snapshot held, when the tool said.
    pub advisory_count: Option<u64>,
}

/// The database state inside a `cargo audit --json` document, when it carries one.
pub fn advisory_database(audit_json: &str) -> Option<AdvisoryDatabase> {
    let audit: Value = serde_json::from_str(audit_json).ok()?;
    let database = audit.get("database")?;
    let last_updated = database.get("last-updated")?.as_str()?.to_string();
    let advisory_count = database.get("advisory-count").and_then(|n| n.as_u64());
    Some(AdvisoryDatabase {
        last_updated,
        advisory_count,
    })
}

impl AdvisoryDatabase {
    /// How old the snapshot is, when Cargo's timestamp can be read.
    ///
    /// `None` rather than a guess when it cannot: a database whose age is unknown is not a
    /// database that is fresh.
    pub fn age(&self, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::Duration> {
        let updated = chrono::DateTime::parse_from_rfc3339(&self.last_updated)
            .ok()?
            .with_timezone(&chrono::Utc);
        Some(now.signed_duration_since(updated))
    }

    /// The line a reader needs, and a note when the age changes what the report can mean.
    ///
    /// The threshold is 30 days: advisories are published continuously, and a month is long
    /// enough that a clean report starts to mean "nothing was published *and old*" rather
    /// than "nothing was published". Below it, the date is stated and nothing is implied.
    pub fn summary(&self, now: chrono::DateTime<chrono::Utc>) -> (String, Option<String>) {
        let counted = match self.advisory_count {
            Some(count) => format!(", {count} advisories"),
            None => ", count unstated".to_string(),
        };
        let Some(age) = self.age(now) else {
            return (
                format!(
                    "advisory database last updated {} ({counted})",
                    self.last_updated
                ),
                Some(format!(
                    "the advisory database's timestamp ({}) could not be read, so how much this check could have seen is unknown — a clean result here says nothing about advisories published since that snapshot",
                    self.last_updated
                )),
            );
        };
        let days = age.num_days();
        let detail = format!(
            "advisory database last updated {} ({days} day(s) old{counted})",
            self.last_updated
        );
        let note = (days > 30).then(|| {
            format!(
                "the advisory database is {days} days old: advisories published since {} are not in this report, so a clean result here is not a clean result today (run `cargo audit` somewhere it can reach the feed to refresh it)",
                self.last_updated
            )
        });
        (detail, note)
    }
}

/// Advisories from `cargo audit --json`.
///
/// The tool is optional in this project (plan §4-D): a caller that cannot run it records
/// a *note*, and this function never sees the difference.
pub fn review_advisories(audit_json: &str) -> Result<Vec<Finding>, String> {
    let audit: Value = serde_json::from_str(audit_json).map_err(|e| format!("cargo audit: {e}"))?;
    let list = audit["vulnerabilities"]["list"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    Ok(list
        .iter()
        .map(|entry| {
            let id = entry["advisory"]["id"].as_str().unwrap_or("RUSTSEC-?");
            let title = entry["advisory"]["title"].as_str().unwrap_or("advisory");
            let raw_severity = entry["advisory"]["severity"]
                .as_str()
                .unwrap_or("medium")
                .to_ascii_lowercase();
            let package = entry["package"]["name"].as_str().unwrap_or("?");
            let version = entry["package"]["version"].as_str().unwrap_or("?");
            // The tool's own scale, collapsed onto ours: `high`/`critical` stop a
            // release, the rest are things to look at. Nothing here is a note.
            let severity = match raw_severity.as_str() {
                "high" | "critical" => Severity::High,
                _ => Severity::Medium,
            };
            let patched = entry["versions"]["patched"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let mut finding = Finding::new(
                format!("advisory-{id}-{package}-{version}"),
                format!("{id}: {title} ({package} {version})"),
                severity,
                "supply-chain",
                format!("RustSec advisory {id} affects {package} {version}."),
            );
            finding.steps.push("Read the advisory for the affected code path.".to_string());
            finding.steps.push(
                "Check whether this project reaches it (a lockfile can pull a crate in unused)."
                    .to_string(),
            );
            if !patched.is_empty() {
                finding
                    .steps
                    .push(format!("Upgrade {package} to one of: {patched}."));
            }
            finding.fix = Some(if patched.is_empty() {
                format!("No patched release is listed for {package} — confirm the affected path is unreachable, or replace the crate.")
            } else {
                format!("Upgrade {package} ({patched}).")
            });
            if let Some(url) = entry["advisory"]["url"].as_str() {
                finding.tags.push(url.to_string());
            }
            finding
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permissive_licence_is_not_an_obligation() {
        assert_eq!(copyleft_in("MIT"), None);
        assert_eq!(copyleft_in("Apache-2.0"), None);
        assert_eq!(copyleft_in("MIT OR Apache-2.0"), None);
    }

    #[test]
    fn a_choice_is_taken_by_its_permissive_branch() {
        // The distinction a naive `contains("GPL")` gets wrong, in the direction that
        // matters: this dependency is usable, and reporting it as copyleft would send
        // someone to replace a crate they are allowed to use.
        assert_eq!(copyleft_in("MIT OR GPL-3.0-only"), None);
        assert_eq!(copyleft_in("Apache-2.0 OR LGPL-2.1"), None);
        // A choice between two copyleft licences is still copyleft.
        assert_eq!(
            copyleft_in("GPL-2.0-only OR GPL-3.0-only"),
            Some(Copyleft::Strong)
        );
    }

    #[test]
    fn a_conjunction_is_decided_by_its_strictest_branch() {
        assert_eq!(copyleft_in("MIT AND GPL-3.0-only"), Some(Copyleft::Strong));
        assert_eq!(copyleft_in("MIT AND MPL-2.0"), Some(Copyleft::Weak));
        assert_eq!(copyleft_in("MIT AND Apache-2.0"), None);
    }

    #[test]
    fn parentheses_do_not_change_the_answer() {
        assert_eq!(copyleft_in("(MIT OR Apache-2.0) AND MIT"), None);
        assert_eq!(
            copyleft_in("(MIT AND (Apache-2.0 AND GPL-3.0-only))"),
            Some(Copyleft::Strong)
        );
    }

    /// A conjunction whose copyleft branch sits inside a choice is still escapable.
    ///
    /// `MIT AND (GPL-2.0-only OR MIT)` asks the consumer to satisfy `MIT` *and* one of
    /// two options — and the second option is MIT, so the copyleft branch can simply be
    /// declined. Reading this as an obligation would tell someone to replace a crate
    /// they are free to use, which is why the assertion is `None` and why it carries
    /// this paragraph: the next reader will see `GPL` in that string and reach for
    /// `contains`.
    #[test]
    fn a_copyleft_branch_inside_a_choice_inside_a_conjunction_is_escapable() {
        assert_eq!(
            copyleft_in("(MIT OR Apache-2.0) AND (GPL-2.0-only OR MIT)"),
            None
        );
    }

    #[test]
    fn weak_and_strong_copyleft_are_told_apart() {
        // Different obligations, different afternoon's work: collapsing them into one
        // "copyleft" red would make the reviewer re-derive which is which.
        assert_eq!(copyleft_in("LGPL-3.0-only"), Some(Copyleft::Weak));
        assert_eq!(copyleft_in("MPL-2.0"), Some(Copyleft::Weak));
        assert_eq!(copyleft_in("AGPL-3.0-only"), Some(Copyleft::Strong));
        assert_eq!(Copyleft::Strong.severity(), Severity::High);
        assert_eq!(Copyleft::Weak.severity(), Severity::Medium);
    }

    const METADATA: &str = r#"{
      "packages": [
        {"name": "firment-core", "version": "0.8.1", "source": null, "license": "MIT"},
        {"name": "anyhow", "version": "1.0.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MIT OR Apache-2.0"},
        {"name": "gpl-thing", "version": "2.1.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "GPL-3.0-only"},
        {"name": "mpl-thing", "version": "1.0.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license": "MPL-2.0"},
        {"name": "mystery", "version": "0.3.0", "source": "registry+https://github.com/rust-lang/crates.io-index", "license_file": "LICENSE"}
      ]
    }"#;

    #[test]
    fn the_report_skips_our_own_crates_and_finds_the_three_kinds() {
        let report = review_metadata(METADATA).unwrap();

        // Four third-party packages, not five: `firment-core` has no source, so it is
        // ours — counting it would pad the distribution with our own licence.
        assert!(
            report
                .details
                .iter()
                .any(|d| d.contains("4 third-party packages"))
        );
        assert!(
            !report.details.iter().any(|d| d.contains("MIT ×2")),
            "got: {:?}",
            report.details
        );

        assert_eq!(report.counts(), (1, 2), "one strong, two medium");
        let titles: Vec<&str> = report.findings.iter().map(|f| f.title.as_str()).collect();
        assert!(
            titles
                .iter()
                .any(|t| t.contains("strong copyleft dependency: gpl-thing"))
        );
        assert!(
            titles
                .iter()
                .any(|t| t.contains("weak copyleft dependency: mpl-thing"))
        );
        assert!(
            titles
                .iter()
                .any(|t| t.contains("licence not declared: mystery"))
        );
        assert!(
            !titles.iter().any(|t| t.contains("anyhow")),
            "a permissive OR choice is not a finding: {titles:?}"
        );
    }

    #[test]
    fn a_metadata_document_with_no_dependencies_is_an_error_not_a_clean_report() {
        // "Nothing to review" and "nothing wrong" must not look the same: the first is
        // a broken invocation, and a report that said "clean" would hide it.
        let empty = r#"{"packages": [{"name": "us", "version": "1", "source": null}]}"#;
        assert!(review_metadata(empty).is_err());
    }

    #[test]
    fn advisories_map_onto_the_two_severities_and_carry_their_fix() {
        let audit = r#"{
          "vulnerabilities": {"found": true, "count": 2, "list": [
            {"advisory": {"id": "RUSTSEC-2026-0001", "title": "bad thing", "severity": "critical", "url": "https://example.test/a"},
             "package": {"name": "badcrate", "version": "0.1.0"},
             "versions": {"patched": [">=0.1.2"]}},
            {"advisory": {"id": "RUSTSEC-2026-0002", "title": "smaller thing", "severity": "low"},
             "package": {"name": "okcrate", "version": "3.0.0"},
             "versions": {"patched": []}}
          ]}
        }"#;
        let findings = review_advisories(audit).unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[1].severity, Severity::Medium);
        assert!(findings[0].fix.as_deref().unwrap().contains(">=0.1.2"));
        // No patched release: the fix says what to do *instead* of promising an upgrade.
        assert!(findings[1].fix.as_deref().unwrap().contains("unreachable"));
        assert_eq!(findings[0].tags, ["https://example.test/a"]);
    }

    const LOCK: &str = r#"
[[package]]
name = "firment-core"
version = "0.8.1"

[[package]]
name = "anyhow"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "gpl-thing"
version = "2.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "never-downloaded"
version = "9.9.9"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn the_lockfile_path_lists_everything_and_admits_what_it_could_not_read() {
        // The fallback that exists because `cargo metadata --offline` fails when any
        // single crate is missing from the cache. Cargo.lock still knows the whole list,
        // so the review runs — and the crate nobody downloaded is *counted as unchecked*
        // rather than quietly dropped, which is the difference between a partial review
        // and a false "clean".
        let mut resolver = |name: &str, version: &str| match (name, version) {
            ("anyhow", _) => Some(LicenseFacts {
                license: Some("MIT OR Apache-2.0".to_string()),
                license_file: false,
            }),
            ("gpl-thing", _) => Some(LicenseFacts {
                license: Some("GPL-3.0-only".to_string()),
                license_file: false,
            }),
            _ => None,
        };
        let report = review_lockfile(LOCK, &mut resolver).unwrap();

        // Three third-party packages: the path dependency has no `source`.
        assert!(
            report
                .details
                .iter()
                .any(|d| d.contains("3 third-party packages"))
        );
        assert_eq!(report.counts().0, 1, "the GPL crate is the high finding");
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.title.contains("strong copyleft dependency: gpl-thing")),
            "got: {:?}",
            report.findings.iter().map(|f| &f.title).collect::<Vec<_>>()
        );
        // The permissive OR choice is not a finding, and the unreadable crate is a
        // finding *plus* a note — the note is what stops `summary()` saying "clean".
        assert_eq!(report.findings.len(), 2, "GPL + one grouped unread finding");
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.title.contains("1 dependencies were not read")),
            "the unread crates are one finding, named for the action: {:?}",
            report.findings.iter().map(|f| &f.title).collect::<Vec<_>>()
        );
        assert_eq!(report.notes.len(), 1);
        assert!(report.notes[0].contains("never-downloaded 9.9.9"));
        assert!(report.summary().contains("1 high"));
    }

    #[test]
    fn a_lockfile_without_dependencies_is_an_error() {
        let lock = "[[package]]\nname = \"us\"\nversion = \"1\"\n";
        let mut resolver = |_: &str, _: &str| None;
        assert!(review_lockfile(lock, &mut resolver).is_err());
    }

    #[test]
    fn the_cache_lookup_reads_a_manifest_and_says_nothing_about_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("index.crates.io-abc123");
        let crate_dir = registry.join("cached-crate-1.2.3");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"cached-crate\"\nversion = \"1.2.3\"\nlicense = \"MIT\"\n",
        )
        .unwrap();

        let facts = cached_license_in(dir.path(), "cached-crate", "1.2.3").unwrap();
        assert_eq!(facts.license.as_deref(), Some("MIT"));
        assert!(!facts.license_file);
        // Not in the cache is `None`, not a guess: the caller turns that into "not read".
        assert!(cached_license_in(dir.path(), "other-crate", "1.0.0").is_none());
    }

    #[test]
    fn the_advisory_database_state_is_read_from_the_audit_json() {
        let audit = r#"{"database":{"advisory-count":1268,"last-commit":"abc","last-updated":"2026-08-11T22:54:25Z"},"vulnerabilities":{"found":false,"count":0,"list":[]}}"#;
        let database = advisory_database(audit).unwrap();
        assert_eq!(database.advisory_count, Some(1268));
        assert_eq!(database.last_updated, "2026-08-11T22:54:25Z");
        // No database object: `None`, not a default that would look fresh.
        assert!(advisory_database(r#"{"vulnerabilities":{}}"#).is_none());
        assert!(advisory_database("not json").is_none());
    }

    #[test]
    fn a_clean_report_says_how_old_the_database_behind_it_was() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-20T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        // The case this was written for: this machine's own advisory database.
        let stale = AdvisoryDatabase {
            last_updated: "2026-08-11T22:54:25Z".to_string(),
            advisory_count: Some(1268),
        };
        let (detail, note) = stale.summary(now);
        assert!(detail.contains("1268 advisories"), "{detail}");
        // 39, not 40: the snapshot is from 22:54 and "now" is midday, so the day count
        // truncates. An age is not a rounding-up kind of number.
        assert!(detail.contains("39 day(s) old"), "{detail}");
        let note = note.expect("an old database earns a note");
        assert!(note.contains("not a clean result today"), "{note}");

        // Recent enough: the age is stated and nothing is implied.
        let fresh = AdvisoryDatabase {
            last_updated: "2026-09-19T00:00:00Z".to_string(),
            advisory_count: Some(1300),
        };
        let (detail, note) = fresh.summary(now);
        assert!(detail.contains("1 day(s) old"), "{detail}");
        assert!(note.is_none(), "{note:?}");

        // An unreadable timestamp is not a fresh database.
        let broken = AdvisoryDatabase {
            last_updated: "yesterday".to_string(),
            advisory_count: None,
        };
        let (detail, note) = broken.summary(now);
        assert!(detail.contains("count unstated"), "{detail}");
        assert!(
            note.unwrap().contains("could not be read"),
            "unreadable is not fresh"
        );
    }

    #[test]
    fn an_empty_advisory_list_is_no_findings() {
        let audit = r#"{"vulnerabilities": {"found": false, "count": 0, "list": []}}"#;
        assert!(review_advisories(audit).unwrap().is_empty());
    }
}
