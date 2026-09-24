//! Diagnostics behind `firm doctor`, `firm --doctor` and `firm --doctor --sbc`.
//!
//! Four stages, printed in this order: provider config + connectivity probe
//! (`doctor`), install location and PATH state (`doctor_install`), local
//! toolchain + serial ports + `[tools]` semantics (`doctor_tools`), and the
//! optional SBC edge-model data plane check (`doctor_sbc`). Nothing here talks
//! to hardware: each check exists so a missing piece fails with a fix hint
//! instead of failing mid-task with a confusing tool error.

use crate::install;
use firment_core::Config;
use firment_core::config::config_path;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The key `doctor` will send, and the label that says where it came from.
/// Both come out of one resolution (`Config::api_key_for`, inline ->
/// auth.json -> env, blank meaning unset) — the status text and the probe
/// request used to resolve independently, so an auth-only provider reported
/// "configured (auth.json)" and then probed with no key at all.
pub(crate) fn doctor_key(
    config: &Config,
    name: &str,
    provider: &firment_core::config::ProviderConfig,
) -> (Option<String>, String) {
    // One resolver in the kernel, so the doctor and `firm config --show` cannot disagree about
    // which source won — they used to be two implementations, and the weaker one could only say
    // "auth.json or the environment".
    let (key, source) = config.resolve_api_key(provider, name);
    (key, source.label())
}

/// Probe every configured provider, printing the detail and returning which ones answered.
///
/// The return value is what `firm doctor`'s closing summary is built from: a reader who runs
/// doctor wants one answer to "can I work right now?", and computing it from the probes that
/// have already happened costs nothing.
pub(crate) async fn doctor(
    config: &Config,
    path: &Path,
    cwd: &Path,
) -> anyhow::Result<Vec<(String, bool)>> {
    println!("config file: {}", path.display());
    // Before the provider check, and deliberately so: a plugin declaration says nothing about
    // providers, and a config with plugins and no provider would otherwise report neither.
    doctor_plugins(config, cwd);
    let mut reachable: Vec<(String, bool)> = Vec::new();
    if config.providers.is_empty() {
        println!("no providers configured");
        return Ok(reachable);
    }
    for (name, provider) in &config.providers {
        let (key, key_status) = doctor_key(config, name, provider);
        println!(
            "provider {name}: type={} model={}",
            provider.r#type, provider.model
        );
        println!("  api key: {key_status}");

        let probe_url = provider.models_url();
        let client = firment_core::http_builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let mut request = client.get(&probe_url);
        if provider.r#type == "anthropic" {
            if let Some(key) = key {
                request = request.header("x-api-key", key);
            }
            request = request.header("anthropic-version", "2023-06-01");
        } else if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        match request.send().await {
            Ok(response) => {
                // A reachable endpoint that answers 401 is still reachable: the network and
                // the URL are fine, and the key is a separate problem the line above names.
                reachable.push((name.clone(), true));
                println!("  probe {probe_url}: HTTP {}", response.status())
            }
            Err(e) => {
                reachable.push((name.clone(), false));
                println!("  probe {probe_url}: unreachable ({e})");
                // reqwest's top-level Display hides the real cause; walk the
                // source chain so the user sees WHAT failed, not just that
                // something did.
                let mut src = std::error::Error::source(&e);
                while let Some(cause) = src {
                    println!("    cause: {cause}");
                    src = cause.source();
                }
                println!(
                    "    hint: if you are behind a proxy, this is more likely a TLS \
                     middlebox (renegotiation / MITM certificate) than a bad URL or \
                     key — retry with HTTPS_PROXY='' to confirm. LAN endpoints need \
                     NO_PROXY."
                );
            }
        }
    }
    Ok(reachable)
}

/// Every declared plugin: the resolved command, whether it exists, and its capabilities.
///
/// Printing the *resolved* path is the point (plugin review §5 step 2). A declaration is a path
/// someone wrote once; a plugin is a path that will be executed. Showing both makes "which file
/// is `./plugins/x.sh`" answerable without reading the config parser, and makes a change to
/// either visible before anything runs.
fn doctor_plugins(config: &Config, base: &Path) {
    if config.plugins.is_empty() {
        return;
    }
    // `base` is the session's cwd, the same directory the merged config was read for and the
    // same one `session_registry` resolves against. One answer, not two: a doctor report that
    // disagreed with what the agent would run would be worse than no report.
    println!("\nplugins:");
    for plugin in firment_core::plugin::declared_plugins(&config.plugins, base) {
        println!(
            "  {} -> {}{}{}",
            plugin.name,
            plugin.path.display(),
            if plugin.path.exists() {
                ""
            } else {
                "  (not found)"
            },
            // The reason a declared plugin is not callable, printed where the declaration is:
            // "it is in the config but the agent cannot use it" is the question this answers.
            if plugin.trusted {
                ""
            } else {
                "  [declared, not enabled — needs trusted = true]"
            }
        );
        if !plugin.args.is_empty() {
            println!("    args: {}", plugin.args.join(" "));
        }
        match &plugin.capabilities {
            Ok(caps) if caps.is_empty() => println!(
                "    capabilities: none declared — the plugin gets nothing but its own stdio"
            ),
            Ok(caps) => println!(
                "    capabilities: {}",
                caps.iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            // A declaration that does not parse is shown as broken rather than as a plugin
            // that is quietly missing.
            Err(e) => println!("    capabilities: INVALID — {e}"),
        }
    }
}

pub(crate) fn doctor_install() {
    let dir = install::default_bin_dir();
    let target = dir.join(install::exe_name());
    let installed = target.is_file();
    let in_path = install::user_path_contains(&dir);
    println!("\ninstall:");
    println!("  bin dir       : {}", dir.display());
    println!(
        "  installed     : {}",
        if installed {
            "yes"
        } else {
            "no (run `firm install`)"
        }
    );
    println!(
        "  PATH includes : {}",
        if in_path {
            "yes"
        } else {
            "no (run `firm install`, then open a new terminal)"
        }
    );
    match std::env::current_exe() {
        Ok(current) => {
            let running_installed = installed
                && std::fs::canonicalize(&current).ok() == std::fs::canonicalize(&target).ok();
            println!(
                "  running from  : {} ({})",
                current.display(),
                if running_installed {
                    "installed copy"
                } else {
                    "other location"
                }
            );
        }
        Err(e) => println!("  running from  : unknown ({e})"),
    }
    println!(
        "  config dir    : {} ({})",
        firment_core::config_dir().display(),
        if firment_core::config_dir().is_dir() {
            "ok"
        } else {
            "not created yet"
        }
    );
}

/// How much a missing check matters.
///
/// The distinction is what lets `doctor` end with a useful exit code: "you have
/// no compiler for your chip" and "you have no logic analyser" are both
/// "not found", but only the first should fail a setup check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Ok,
    /// Not installed, and nothing works without it.
    Required,
    /// Not installed; only the matching feature is unavailable.
    Optional,
}

impl State {
    /// Whether this state means the machine is not ready. Drives the exit code.
    fn is_blocking(self) -> bool {
        self == State::Required
    }
}

/// One toolchain probe, kept as data so the text and `--json` views cannot
/// disagree about what was checked or how it came out.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Check {
    pub name: String,
    pub state: State,
    /// What it is for, or the version when found.
    pub detail: String,
    /// The exact command that fixes it, per platform. Empty when found.
    pub fix: String,
}

impl Check {
    fn found(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            state: State::Ok,
            detail: detail.into(),
            fix: String::new(),
        }
    }

    fn missing(
        name: &str,
        state: State,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            name: name.to_string(),
            state,
            detail: detail.into(),
            fix: fix.into(),
        }
    }
}

/// The per-platform install command for a missing tool.
///
/// Windows names the actual manager or download page rather than `winget
/// install <guess>`: a wrong package id is worse than no hint, because it looks
/// authoritative and fails.
/// The runtime the **GUI** needs and the CLI does not (plan §6's list).
///
/// Two decisions here, both about not turning a check into a scare:
///
/// * **Windows only.** On any other platform the question has no answer this machine can
///   look up, and a guess printed as a check is worse than no line at all.
/// * **Never `Required`.** Someone who only runs the CLI does not need a webview; a red
///   "required" line for something the reader is not using is exactly what §16.4 is about.
///   `Optional` says "this is what the GUI would need" and leaves the exit code alone.
#[cfg(windows)]
fn gui_runtime_checks() -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(match webview2_version() {
        Some(version) => Check::found("WebView2 runtime", format!("GUI webview — {version}")),
        None => Check::missing(
            "WebView2 runtime",
            State::Optional,
            "the webview the GUI renders in",
            "winget install Microsoft.EdgeWebView2Runtime   (installing Edge brings it too)",
        ),
    });

    checks.push(if msvc_runtime_present() {
        Check::found("MSVC runtime", "GUI binary dependency — vcruntime140.dll")
    } else {
        Check::missing(
            "MSVC runtime",
            State::Optional,
            "the C runtime the GUI binary links against",
            "winget install Microsoft.VCRedist.2015+.x64",
        )
    });

    checks
}

/// Nothing to check off Windows, and saying so with an empty list is more honest than a
/// line that would have to guess.
#[cfg(not(windows))]
fn gui_runtime_checks() -> Vec<Check> {
    Vec::new()
}

/// The installed WebView2 runtime's version, from the directory Edge maintains.
///
/// The **directory**, not the registry. The obvious probe is `reg query` on Edge's
/// `EdgeUpdate\Clients\{…}` key, and it was the first version of this function — until
/// running it here answered that `reg.exe` is on the sandbox's program blacklist. A check
/// that cannot run where the product runs is not a check.
///
/// Version-named subdirectories under `EdgeWebView\Application` are what the installers
/// leave behind, and the highest is the one that would actually load. Both locations are
/// searched — the machine-wide install and the per-user one — and the literal
/// `Program Files (x86)` path is included because an environment without the
/// `ProgramFiles(x86)` variable would otherwise report a runtime that is plainly there.
#[cfg(windows)]
fn webview2_version() -> Option<String> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for var in [
        "ProgramFiles(x86)",
        "PROGRAMFILES",
        "ProgramW6432",
        "LOCALAPPDATA",
    ] {
        if let Some(root) = std::env::var_os(var) {
            roots.push(std::path::PathBuf::from(root));
        }
    }
    roots.push(std::path::PathBuf::from(r"C:\Program Files (x86)"));
    roots.push(std::path::PathBuf::from(r"C:\Program Files"));

    let mut found: Vec<String> = Vec::new();
    for root in roots {
        let dir = root
            .join("Microsoft")
            .join("EdgeWebView")
            .join("Application");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // A version directory starts with a digit; the installer also leaves
            // `SetupMetrics`-style entries beside them.
            if name.chars().next().is_some_and(|c| c.is_ascii_digit()) && entry.path().is_dir() {
                found.push(name);
            }
        }
    }

    // Compare numerically: string order puts `99.0` above `123.0`, and an installed-but-old
    // runtime is not the one the GUI would load.
    found.sort_by_key(|version| {
        let mut parts: Vec<u64> = version
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect();
        parts.resize(4, 0);
        (parts[0], parts[1], parts[2], parts[3])
    });
    found.pop()
}

/// Whether the MSVC C runtime is on this machine.
///
/// The DLL rather than a registry entry: the GUI binary links against it by name, so its
/// presence is the fact that matters, and a redistributable entry would still be a proxy.
#[cfg(windows)]
fn msvc_runtime_present() -> bool {
    std::env::var_os("SystemRoot")
        .map(|root| {
            std::path::PathBuf::from(root)
                .join("System32")
                .join("vcruntime140.dll")
        })
        .is_some_and(|dll| dll.exists())
}

fn install_hint(what: &str) -> &'static str {
    match what {
        "rust" => {
            if cfg!(windows) {
                "winget install Rustlang.Rustup   (or https://rustup.rs)"
            } else {
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
            }
        }
        "arm-gcc" => {
            if cfg!(windows) {
                "winget install Arm.GnuArmEmbeddedToolchain   (or the xPack build from \
                 https://github.com/xpack-dev-tools/arm-none-eabi-gcc-xpack/releases)"
            } else if cfg!(target_os = "macos") {
                "brew install --cask gcc-arm-embedded"
            } else {
                "sudo apt install gcc-arm-none-eabi   (or your distro's equivalent)"
            }
        }
        "probe-rs" => "cargo install probe-rs-tools   (needed by flash / run)",
        "openocd" => {
            if cfg!(windows) {
                "download from https://github.com/openocd-org/openocd/releases, or install the \
                 xPack build"
            } else if cfg!(target_os = "macos") {
                "brew install open-ocd"
            } else {
                "sudo apt install openocd"
            }
        }
        "stlink" => {
            if cfg!(windows) {
                "winget install stlink   (ST's STM32CubeProgrammer also ships st-link tools)"
            } else if cfg!(target_os = "macos") {
                "brew install stlink"
            } else {
                "sudo apt install stlink-tools"
            }
        }
        "stm32flash" => {
            if cfg!(windows) {
                "get stm32flash.exe from https://sourceforge.net/projects/stm32flash/ and put it \
                 on PATH"
            } else {
                "sudo apt install stm32flash"
            }
        }
        "sigrok" => {
            if cfg!(windows) {
                "the self-extracting package from https://sigrok.org/wiki/Downloads, then add it \
                 to PATH or point [tools.la] bin at sigrok-cli.exe"
            } else if cfg!(target_os = "macos") {
                "brew install sigrok-cli"
            } else {
                "sudo apt install sigrok-cli"
            }
        }
        "node" => "https://nodejs.org (the web client is a Next.js app)",
        "python" => "https://python.org (needed by PlatformIO's tooling)",
        "git" => "https://git-scm.com (needed for the workbench branch view)",
        _ => "",
    }
}

/// Probes one command with `--version` and returns its first output line.
///
/// Used only for tools that are safe to execute; `which` covers the ones with
/// GUI side effects (see its doc).
fn version_of(bin: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = if out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    Some(line.chars().take(80).collect())
}

/// Whether this directory looks like a firmware project, which is what makes
/// the cross-compiler REQUIRED rather than merely useful.
///
/// The same binary is used on a docs repo and on a CubeMX checkout; failing
/// `doctor` for a missing `arm-none-eabi-gcc` in the first case would be wrong.
fn looks_like_firmware_project(cwd: &Path) -> bool {
    const MARKERS: &[&str] = &[
        "platformio.ini",
        "Makefile",
        "makefile",
        "CMakeLists.txt",
        "STM32CubeMX",
    ];
    if MARKERS.iter().any(|m| cwd.join(m).is_file()) {
        return true;
    }
    // Any *.uvprojx / *.ioc at the top level is decisive on its own.
    let Ok(entries) = std::fs::read_dir(cwd) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        name.ends_with(".uvprojx") || name.ends_with(".ioc")
    })
}

/// The toolchain block: everything a firmware project may need, each with the
/// command that installs it.
///
/// Required-vs-optional is decided by `cwd`: a cross-compiler is required in a
/// firmware project and optional everywhere else, so `doctor` can be run in a
/// docs repo without failing.
pub(crate) fn toolchain_checks(
    cwd: &Path,
    tools: &firment_core::config::ToolsConfig,
) -> Vec<Check> {
    let firmware = looks_like_firmware_project(cwd);
    let mut checks = Vec::new();

    // The one thing that is required regardless of project type: `firm` is a
    // Rust binary and its build/probe helpers are cargo subcommands.
    match version_of("rustc", &["--version"]) {
        Some(v) => checks.push(Check::found("rustc", v)),
        None => checks.push(Check::missing(
            "rustc",
            State::Required,
            "Rust toolchain",
            install_hint("rust"),
        )),
    }

    for (bin, what, detail) in [
        (
            "arm-none-eabi-gcc",
            "arm-gcc",
            "ARM cross-compiler for bare-metal C/C++",
        ),
        (
            "probe-rs",
            "probe-rs",
            "flashing and running (flash/run tools)",
        ),
        (
            "openocd",
            "openocd",
            "alternative debug-probe driver (GDB server)",
        ),
        ("stlink", "stlink", "ST-Link CLI — alternative flasher"),
        ("stm32flash", "stm32flash", "serial-bootloader flasher"),
        ("git", "git", "version control (workbench branch view)"),
        ("node", "node", "web client build (Next.js)"),
        ("python", "python", "PlatformIO and vendor tooling"),
    ] {
        // Only the cross-compiler is promoted by project type; a probe driver
        // is optional because `la`/`observe` can be the whole workflow.
        let state = if what == "arm-gcc" && firmware {
            State::Required
        } else {
            State::Optional
        };
        match version_of(bin, &["--version"]) {
            Some(v) => checks.push(Check::found(bin, v)),
            None => checks.push(Check::missing(bin, state, detail, install_hint(what))),
        }
    }

    // Build systems, via PATH only: Keil's uv4 opens a GUI when invoked bare,
    // and doctor must never open a window.
    for (bin, what) in [
        ("pio", "PlatformIO CLI — platformio.ini projects"),
        ("cmake", "CMake — CMakeLists.txt projects"),
        ("make", "GNU make — Makefile projects"),
        ("uv4", "Keil MDK uVision — *.uvprojx projects"),
    ] {
        match which(bin) {
            Some(_) => checks.push(Check::found(bin, what)),
            None => checks.push(Check::missing(bin, State::Optional, what, String::new())),
        }
    }

    // Logic analyser, honouring a configured binary name.
    let sigrok_bin = tools
        .la
        .as_ref()
        .and_then(|la| la.bin.clone())
        .unwrap_or_else(|| "sigrok-cli".to_string());
    match version_of(&sigrok_bin, &["--version"]) {
        Some(v) => checks.push(Check::found(&sigrok_bin, v)),
        None => checks.push(Check::missing(
            &sigrok_bin,
            State::Optional,
            "the la tool — logic captures and waveform measurements",
            install_hint("sigrok"),
        )),
    }

    // The GUI's runtime, last because it is the only section a CLI-only user can ignore
    // (plan §6 lists it; the CLI does not need a webview).
    checks.extend(gui_runtime_checks());

    checks
}

/// Local model servers (plan §5, item 5) — probed **here and in `firm config`, nowhere
/// else** (§16.2-6), with a 200 ms timeout and a 15-minute cache.
///
/// The cache is why a second `firm doctor` in the same session costs nothing, and the
/// expiry is why starting Ollama and asking again works without anyone knowing a cache
/// exists.
pub(crate) async fn doctor_local(config: &Config) -> Vec<firment_core::local::LocalEndpoint> {
    use firment_core::local;

    println!("\nlocal model servers:");
    let now = local::now_secs();
    let (endpoints, from_cache) = match local::cached(now) {
        Some(endpoints) => (endpoints, true),
        None => {
            let extra: Vec<String> = config
                .providers
                .values()
                .filter_map(|p| p.base_url.clone())
                .filter(|url| local::is_private_url(url))
                .collect();
            let probed = local::probe_with(&extra).await;
            local::store(&probed, now);
            (probed, false)
        }
    };

    if endpoints.is_empty() {
        let ports: Vec<String> = local::KNOWN_ENDPOINTS
            .iter()
            .map(|(_, display, port)| format!("{display} on {port}"))
            .collect();
        println!("  none listening ({})", ports.join(", "));
        return Vec::new();
    }
    println!(
        "  (from {})",
        if from_cache { "cache" } else { "a fresh probe" }
    );

    for endpoint in &endpoints {
        println!("  {} at {}", endpoint.kind, endpoint.base_url);
        if endpoint.models.is_empty() {
            println!("    reachable, but it listed no models");
            continue;
        }
        for model in local::fits(&endpoint.models) {
            match model.needs_gb {
                Some(needs) => println!("    {} (~{needs:.1} GB)", model.name),
                None => println!("    {} (size unknown)", model.name),
            }
        }
        match config.local.vram_gb {
            Some(vram) => {
                let ranked = local::recommend(&endpoint.models, Some(vram));
                if ranked.is_empty() {
                    println!("    nothing here fits {vram:.0} GB of VRAM");
                } else {
                    println!(
                        "    best fit for {vram:.0} GB: {} (set [local] vram_gb to change this)",
                        ranked[0].name
                    );
                }
            }
            // No number, no recommendation: a "fits" verdict computed from an assumed card
            // is worse than silence.
            None => println!(
                "    set [local] vram_gb in config.toml to get a recommendation for this machine"
            ),
        }
    }
    endpoints
}

/// The closing answer: **can I work right now?**
///
/// `firm doctor` already reports each provider, each local server and the toolchain, but a
/// reader has to combine five sections to answer the one question they came with. An offline
/// machine makes that obvious: the interesting fact is not "this provider is unreachable", it
/// is "chat needs the network and everything else does not".
///
/// Pure, so the wording is testable without a network — which is the point of the section it
/// produces.
pub(crate) fn capabilities_summary(
    providers: &[(String, bool)],
    locals: &[firment_core::local::LocalEndpoint],
    mqtt: Option<bool>,
) -> String {
    let mut out = String::from("\ncan I work right now?\n");
    if providers.is_empty() {
        out.push_str("  model chat    : no providers configured\n");
    } else {
        let up: Vec<&str> = providers
            .iter()
            .filter(|(_, ok)| *ok)
            .map(|(name, _)| name.as_str())
            .collect();
        let down: Vec<&str> = providers
            .iter()
            .filter(|(_, ok)| !*ok)
            .map(|(name, _)| name.as_str())
            .collect();
        out.push_str(&format!(
            "  model chat    : {}\n",
            if up.is_empty() {
                "nothing reachable — model-backed work needs the network (or a LAN server)"
                    .to_string()
            } else {
                format!("{} reachable", up.join(", "))
            }
        ));
        if !down.is_empty() {
            out.push_str(&format!(
                "                  unreachable: {}\n",
                down.join(", ")
            ));
        }
    }
    out.push_str(&format!(
        "  local models  : {}\n",
        if locals.is_empty() {
            "none listening (Ollama 11434, llama.cpp 8080, LM Studio 1234)".to_string()
        } else {
            locals
                .iter()
                .map(|endpoint| format!("{} at {}", endpoint.kind, endpoint.base_url))
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    out.push_str(&format!(
        "  device plane  : {}\n",
        match mqtt {
            Some(true) => "broker reachable",
            Some(false) => "broker unreachable — device alerts and commands are off",
            None => "not configured (no [mqtt] broker)",
        }
    ));
    // The part that is true regardless of the network, so that an offline reader knows the
    // answer is not "nothing works".
    out.push_str(
        "  no network    : sessions, edits, build/flash/monitor, the static rules review, \
         replay, share and ADRs all work offline\n",
    );
    out
}

/// [`capabilities_summary`] with the device-plane probe done first.
pub(crate) async fn capabilities(
    providers: &[(String, bool)],
    locals: &[firment_core::local::LocalEndpoint],
    config: &Config,
) -> String {
    let broker = config.mqtt.broker.trim();
    let mqtt = if broker.is_empty() {
        None
    } else {
        Some(mqtt_reachable(broker).await)
    };
    capabilities_summary(providers, locals, mqtt)
}

/// Whether the configured MQTT broker answers, with a short deadline.
///
/// Runs on a **blocking thread**, and that is not a detail: `rumqttc`'s blocking client owns
/// a runtime of its own, and dropping one inside an async context panics with "Cannot drop a
/// runtime in a context where blocking is not allowed" — which is exactly what running
/// `firm --doctor` did, once, before this comment existed. The guard loop gets away with the
/// same client because it never drops it mid-turn.
async fn mqtt_reachable(broker: &str) -> bool {
    let broker = broker.to_string();
    tokio::task::spawn_blocking(move || mqtt_reachable_blocking(&broker))
        .await
        .unwrap_or(false)
}

/// The probe itself: connect, and treat the CONNACK as the answer.
fn mqtt_reachable_blocking(broker: &str) -> bool {
    let (host, port) = match broker.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(1883)),
        None => (broker.to_string(), 1883),
    };
    let opts = rumqttc::MqttOptions::new("firm-doctor", &host, port);
    let (_client, mut connection) = rumqttc::Client::new(opts, 8);
    // `try_recv` in a bounded loop: without the `eventloop` feature the connection hands its
    // events over a channel that a background thread fills. The answer that means "there is a
    // broker here" is the CONNACK; anything else — refused, timed out, something unexpected —
    // is a "no" for this line, and not an error worth propagating.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match connection.try_recv() {
            Ok(Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_)))) => return true,
            // Other traffic before the CONNACK: the broker is talking, keep waiting for it.
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(rumqttc::TryRecvError::Disconnected) => return false,
            Err(rumqttc::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// The first check that must be installed for `firm` to do its job, if any.
///
/// Drives the exit code: `doctor` is a setup gate, so it has to be able to say
/// "this machine is not ready" in a way a script can read. Warnings deliberately
/// do not count.
pub(crate) fn first_required_missing(checks: &[Check]) -> Option<String> {
    checks
        .iter()
        .find(|c| c.state.is_blocking())
        .map(|c| c.name.clone())
}

/// Minimal PATH lookup without execution. Used for toolchain checks instead
/// of running each tool with `--version`: some (Keil's uv4) have GUI side
/// effects when invoked bare, and doctor must never open windows.
fn which(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![String::new()]
    };
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Whether the first token of a user-configured build_command can resolve.
/// cmd.exe shell builtins are accepted unconditionally (they never resolve
/// via PATH but always work inside `cmd /C`).
fn build_command_resolves(command: &str) -> bool {
    const CMD_BUILTINS: &[&str] = &[
        "cd", "echo", "dir", "del", "copy", "move", "md", "rd", "call", "set", "type", "exit",
        "if", "for", "rem",
    ];
    let first = command
        .split(|c: char| c.is_whitespace() || c == '&' || c == '|')
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .trim_matches('"');
    if first.is_empty() {
        return false;
    }
    if first.contains('/') || first.contains('\\') {
        Path::new(first).is_file()
    } else if cfg!(windows) && CMD_BUILTINS.contains(&first.to_ascii_lowercase().as_str()) {
        true
    } else {
        which(first).is_some()
    }
}

/// Toolchain + serial + `[tools]` semantics checks. This never talks to
/// hardware: it verifies what build/flash/monitor need so a missing piece
/// fails HERE with a fix hint instead of mid-task with a confusing error.
/// detect_build_command only looks for manifest FILES — doctor is the first
/// place that checks whether the toolchain binaries themselves exist.
pub(crate) fn doctor_tools(
    cwd: &Path,
    tools: &firment_core::config::ToolsConfig,
    as_json: bool,
) -> Vec<Check> {
    let checks = toolchain_checks(cwd, tools);
    if as_json {
        // Only the machine-readable block goes to stdout when --json is set:
        // a caller parsing it must not have to strip human prose first.
        return checks;
    }
    print_toolchain(&checks);

    println!("\nserial ports:");
    let ports = firment_tools::tools::monitor::enumerate_ports();
    println!("  {ports}");

    print_tools_config(cwd, tools);
    checks
}

/// The toolchain block, one line per probe, each missing one followed by the
/// command that installs it.
fn print_toolchain(checks: &[Check]) {
    println!("\ntoolchain:");
    for check in checks {
        let marker = match check.state {
            State::Ok => "✓",
            State::Required => "✗",
            State::Optional => "·",
        };
        let suffix = if check.state == State::Required {
            " — REQUIRED"
        } else {
            ""
        };
        println!("  {:<18} {marker} {}{suffix}", check.name, check.detail);
        if !check.fix.is_empty() {
            println!("  {:<18}   fix: {}", "", check.fix);
        }
    }
}

fn print_tools_config(cwd: &Path, tools: &firment_core::config::ToolsConfig) {
    println!("\n[tools] config ({}):", cwd.display());
    match &tools.default_chip {
        Some(chip) => println!("  default_chip : {chip}"),
        None => {
            println!("  default_chip : not set — flash/run then require an explicit chip parameter")
        }
    }
    match &tools.monitor_port {
        Some(port) => {
            // Exact name match against bare port names: substring-matching
            // the joined label string reported COM1 as "present" whenever
            // COM10 existed.
            let attached = firment_tools::tools::monitor::port_names()
                .iter()
                .any(|p| p == port);
            println!(
                "  monitor_port : {port} — {}",
                if attached {
                    "present"
                } else {
                    "NOT attached right now (unplugged, or a stale config entry?)"
                }
            );
        }
        None => println!("  monitor_port : not set — monitor will ask to pick one"),
    }
    println!("  monitor_baud : {}", tools.monitor_baud);
    match &tools.la {
        Some(la) => println!(
            "  la           : driver={} samplerate={} channels={}",
            la.driver,
            la.samplerate.as_deref().unwrap_or("(device default)"),
            la.channels.as_deref().unwrap_or("(all)"),
        ),
        None => println!(
            "  la           : not configured — la capture then needs an explicit driver/channels \
             per call"
        ),
    }
    match &tools.build_command {
        Some(cmd) => println!(
            "  build_command: {}",
            if build_command_resolves(cmd) {
                format!("\"{cmd}\" — resolves")
            } else {
                format!("\"{cmd}\" — first token NOT found on PATH (build would fail)")
            }
        ),
        None => println!(
            "  build_command: not set — build auto-detects platformio.ini / Makefile / \
             CMakeLists.txt / *.uvprojx"
        ),
    }
}

/// `firm --doctor --sbc`: end-to-end check of the SBC edge-model data plane.
/// Every failing stage stops the chain with a concrete fix hint — the point
/// is to answer "is the SBC side actually set up?" without reading logs.
pub(crate) async fn doctor_sbc(config: &Config) {
    println!("\nsbc edge-model checks:");

    // Stage 1 — [mqtt] broker must be configured and parseable.
    let broker = config.mqtt.broker.trim().to_string();
    let Some((host, port)) = parse_host_port(&broker) else {
        println!("  ✗ [mqtt] broker missing or invalid (got {broker:?})");
        println!("    hint: add to {} :", config_path().display());
        println!("      [mqtt]");
        println!("      broker = \"<sbc-ip>:1883\"   # mosquitto on the SBC");
        return;
    };
    println!("  ✓ [mqtt] broker = {host}:{port}");

    // Stage 2 — TCP reachability, with distinct refused vs timeout hints.
    match tokio::time::timeout(
        Duration::from_secs(4),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(_)) => println!("  ✓ tcp {host}:{port} reachable"),
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::ConnectionRefused {
                println!("  ✗ tcp {host}:{port} connection refused");
                println!("    hint: mosquitto not running on the SBC:");
                println!("          ssh <user>@{host} -- sudo systemctl status mosquitto");
            } else {
                println!("  ✗ tcp {host}:{port}: {e}");
                println!(
                    "    hint: wrong IP or firewall; pin the SBC IP in the router's DHCP reservations"
                );
            }
            return;
        }
        Err(_) => {
            println!("  ✗ tcp {host}:{port} timed out after 4s");
            println!("    hint: host unreachable (wrong IP? SBC offline? wifi down?)");
            return;
        }
    }

    // Stage 3+4 — one MQTT session: CONNACK, then grab the retained guard
    // heartbeat from firment/guard/status.
    let mut opts = rumqttc::MqttOptions::new("firm-doctor", &host, port);
    opts.set_clean_session(true);
    opts.set_keep_alive(Duration::from_secs(10));
    let (client, mut eventloop) = rumqttc::AsyncClient::new(opts, 16);
    client
        .subscribe("firment/guard/status", rumqttc::QoS::AtLeastOnce)
        .await
        .ok();

    let mut connack = false;
    let mut mqtt_err: Option<String> = None;
    let mut guard_status: Option<String> = None;
    _ = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => connack = true,
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p)))
                    if p.topic == "firment/guard/status" =>
                {
                    guard_status = Some(String::from_utf8_lossy(&p.payload).into_owned());
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    mqtt_err = Some(e.to_string());
                    break;
                }
            }
        }
    })
    .await;
    client.disconnect().await.ok();

    if !connack {
        let detail = mqtt_err.map(|e| format!(" ({e})")).unwrap_or_default();
        println!("  ✗ mqtt CONNACK failed{detail}");
        println!("    hint: is that port really mosquitto? check listener + allow_anonymous");
        return;
    }
    println!("  ✓ mqtt CONNACK");

    match guard_status {
        None => {
            println!("  ✗ no retained firment/guard/status within 6s");
            println!(
                "    hint: guardd not installed/running on the SBC — see sbc-guard/README.md;"
            );
            println!(
                "          quick fix: ssh <user>@{host} -- sudo systemctl enable --now firment-guard"
            );
        }
        Some(json) => match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(v) => {
                let ts = v.get("ts").and_then(|x| x.as_i64());
                let beat_min = v
                    .get("standby_minutes")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(10);
                let rules = v.get("rules").and_then(|x| x.as_u64());
                match ts {
                    Some(ts) => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        let age = (now - ts).max(0);
                        if age <= (beat_min * 120 + 60) as i64 {
                            println!("  ✓ guard heartbeat fresh (age {age}s, beat {beat_min}min)");
                        } else {
                            println!(
                                "  ✗ guard heartbeat STALE (age {age}s > 2×beat {beat_min}min)"
                            );
                            println!(
                                "    hint: guardd died after its last beat — journalctl -u firment-guard on the SBC"
                            );
                        }
                    }
                    None => println!(
                        "  ⚠ guard status has no ts field (old guardd build); rules={} — upgrade when convenient",
                        rules.unwrap_or(0)
                    ),
                }
            }
            Err(_) => {
                println!("  ⚠ retained guard frame is not valid JSON — unexpected publisher?")
            }
        },
    }

    // Stage 5 — find provider(s) whose base_url points at the broker host:
    // that is our sbc model endpoint by convention. Verify /models lists the
    // configured model (catches "ollama up but model never pulled").
    let matches: Vec<_> = config
        .providers
        .iter()
        .filter(|(_, p)| {
            p.r#type != "anthropic"
                && url_host(p.base_url.as_deref().unwrap_or("")) == Some(host.as_str())
        })
        .collect();
    if matches.is_empty() {
        println!("  ⚠ no openai-compatible provider points at {host}");
        println!("    hint: add a provider whose base_url is http://{host}:<ollama-port>/v1,");
        println!("          e.g. [providers.sbc-ollama] with type=\"openai\"");
    } else {
        let http = firment_core::http_builder()
            .timeout(Duration::from_secs(8))
            .build()
            .ok();
        for (name, p) in &matches {
            let base = p
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".to_string())
                .trim_end_matches('/')
                .to_string();
            let key = config.api_key_for(p, name);
            let Some(http) = http.clone() else {
                println!("  ⚠ {name}: could not build HTTP client");
                continue;
            };
            let mut req = http.get(format!("{base}/models"));
            if let Some(key) = key {
                req = req.bearer_auth(key);
            }
            match tokio::time::timeout(Duration::from_secs(10), req.send()).await {
                Ok(Ok(resp)) => {
                    let ids: Vec<String> = resp
                        .json::<serde_json::Value>()
                        .await
                        .ok()
                        .and_then(|v| {
                            Some(
                                v.get("data")?
                                    .as_array()?
                                    .iter()
                                    .filter_map(|m| m.get("id")?.as_str().map(String::from))
                                    .collect(),
                            )
                        })
                        .unwrap_or_default();
                    if ids.is_empty() {
                        println!("  ⚠ {name}: {base}/models answered but listed no models");
                    } else if ids.iter().any(|id| id == &p.model) {
                        println!(
                            "  ✓ {name}: model '{}' ready ({} served)",
                            p.model,
                            ids.len()
                        );
                    } else {
                        println!(
                            "  ✗ {name}: model '{}' NOT pulled (endpoint serves: {})",
                            p.model,
                            ids.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                        );
                        println!("    hint: ssh <user>@{host} -- ollama pull {}", p.model);
                    }
                }
                Ok(Err(e)) => {
                    println!("  ✗ {name}: {base}/models unreachable ({e})");
                    println!(
                        "    hint: ollama not running on the SBC: ssh <user>@{host} -- systemctl status ollama"
                    );
                }
                Err(_) => {
                    println!(
                        "  ✗ {name}: {base}/models timed out (cold start can take ~70s; retry once)"
                    );
                }
            }
        }
    }

    // Stage 6 — bound devices from the project's workbench.toml (if any).
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match firment_core::WorkbenchConfig::load(&cwd) {
        Ok(wb) if !wb.devices.is_empty() => {
            let nodes: Vec<String> = wb.devices.keys().cloned().collect();
            println!("  ✓ devices bound: {}", nodes.join(", "));
        }
        Ok(_) => println!("  · workbench.toml has no [devices] — no nodes bound yet"),
        Err(_) => {
            println!("  · no workbench.toml in cwd — device list skipped (run from project root)")
        }
    }
}

/// Split "host:port" (port optional → 1883).
fn parse_host_port(broker: &str) -> Option<(String, u16)> {
    let broker = broker.trim();
    if broker.is_empty() {
        return None;
    }
    match broker.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            Some((h.to_string(), p.parse().ok()?))
        }
        Some((h, _)) if !h.is_empty() => Some((h.to_string(), 1883)),
        _ => Some((broker.to_string(), 1883)),
    }
}

/// Host component of an http(s) URL (None for empty/unparseable input).
fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest.split(['/', ':']).next().filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn the_gui_runtime_checks_exist_and_can_never_block_a_machine() {
        // Two facts pinned. The probes exist (plan §6 lists them), and neither can make
        // doctor fail: someone who only runs the CLI does not need a webview, so a missing
        // one must be a line in the report, not an exit code.
        let checks = gui_runtime_checks();
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"WebView2 runtime"), "{names:?}");
        assert!(names.contains(&"MSVC runtime"), "{names:?}");
        assert!(
            checks.iter().all(|check| !check.state.is_blocking()),
            "the GUI's runtime is optional for a CLI user: {names:?}"
        );
    }

    use super::*;
    use tempfile::tempdir;

    /// The gate has to distinguish "you are missing a cross-compiler in a
    /// firmware checkout" from "you are missing a logic analyser", or `doctor`
    /// is useless as a setup check: one blocks, the other is a note.
    #[test]
    fn only_required_gaps_block() {
        let checks = vec![
            Check::found("rustc", "1.97"),
            Check::missing(
                "sigrok-cli",
                State::Optional,
                "the la tool",
                "brew install sigrok-cli",
            ),
        ];
        assert_eq!(first_required_missing(&checks), None);

        let checks = vec![
            Check::found("rustc", "1.97"),
            Check::missing(
                "arm-none-eabi-gcc",
                State::Required,
                "cross-compiler",
                "apt install x",
            ),
        ];
        assert_eq!(
            first_required_missing(&checks).as_deref(),
            Some("arm-none-eabi-gcc")
        );
    }

    #[test]
    fn a_firmware_checkout_promotes_the_cross_compiler() {
        // An empty directory is not a firmware project: nothing is required
        // there beyond the Rust toolchain, so doctor must not fail on a docs
        // repo.
        let plain = tempdir().unwrap();
        assert!(!looks_like_firmware_project(plain.path()));

        for marker in ["platformio.ini", "Makefile", "CMakeLists.txt"] {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join(marker), "").unwrap();
            assert!(
                looks_like_firmware_project(dir.path()),
                "{marker} marks a firmware project"
            );
        }

        let cubemx = tempdir().unwrap();
        std::fs::write(cubemx.path().join("blink.ioc"), "").unwrap();
        assert!(looks_like_firmware_project(cubemx.path()));

        let keil = tempdir().unwrap();
        std::fs::write(keil.path().join("project.uvprojx"), "").unwrap();
        assert!(looks_like_firmware_project(keil.path()));
    }

    /// A missing tool with no install command is a dead end for the user, which
    /// is the whole reason the hints exist. Every key `install_hint` knows must
    /// answer, and every hint must be a real command rather than a placeholder.
    #[test]
    fn every_probed_tool_can_name_its_fix() {
        for what in [
            "rust",
            "arm-gcc",
            "probe-rs",
            "openocd",
            "stlink",
            "stm32flash",
            "sigrok",
            "node",
            "python",
            "git",
        ] {
            let hint = install_hint(what);
            assert!(!hint.is_empty(), "{what} has no install hint");
            assert!(
                hint.contains("install")
                    || hint.contains("http")
                    || hint.contains("brew")
                    || hint.contains("download")
                    || hint.contains("get "),
                "{what} hint is not actionable: {hint}"
            );
        }
        // An unknown key is not a crash, just no hint.
        assert_eq!(install_hint("no-such-tool"), "");
    }

    /// The probes must actually cover what the plan promises, and each name
    /// must be unique so the JSON report cannot contain the same row twice.
    #[test]
    fn the_probe_list_covers_the_documented_set() {
        let dir = tempdir().unwrap();
        let checks = toolchain_checks(dir.path(), &firment_core::config::ToolsConfig::default());
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        for expected in [
            "rustc",
            "arm-none-eabi-gcc",
            "probe-rs",
            "openocd",
            "stlink",
            "stm32flash",
            "git",
            "node",
            "python",
            "pio",
            "cmake",
            "make",
            "uv4",
            "sigrok-cli",
        ] {
            assert!(names.contains(&expected), "missing probe: {expected}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            names.len(),
            "duplicate probe names: {names:?}"
        );

        // A non-firmware dir must not mark anything REQUIRED: the runner would
        // otherwise exit 2 on a machine that can do everything it was asked to.
        assert!(
            checks.iter().all(|c| c.state != State::Required),
            "nothing should be required outside a firmware project"
        );
    }

    /// In a firmware checkout the cross-compiler blocks, so a CI setup step can
    /// gate on `doctor`'s exit code.
    #[test]
    fn a_firmware_checkout_marks_a_missing_cross_compiler_required() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("platformio.ini"), "").unwrap();
        let checks = toolchain_checks(dir.path(), &firment_core::config::ToolsConfig::default());
        let gcc = checks
            .iter()
            .find(|c| c.name == "arm-none-eabi-gcc")
            .expect("the cross-compiler is probed");
        // On a machine that HAS the toolchain this is Ok; either way the state
        // must be one of the two that mean "this matters here".
        assert!(
            matches!(gcc.state, State::Ok | State::Required),
            "in a firmware project the cross-compiler cannot be merely optional, got {:?}",
            gcc.state
        );
        if gcc.state == State::Ok {
            assert!(!gcc.detail.is_empty(), "a found tool reports its version");
        } else {
            assert!(!gcc.fix.is_empty(), "a required gap must name its fix");
        }
    }
}
