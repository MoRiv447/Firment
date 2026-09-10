//! Diagnostics behind `firm doctor`, `firm --doctor` and `firm --doctor --sbc`.
//!
//! Four stages, printed in this order: provider config + connectivity probe
//! (`doctor`), install location and PATH state (`doctor_install`), local
//! toolchain + serial ports + `[tools]` semantics (`doctor_tools`), and the
//! optional SBC edge-model data plane check (`doctor_sbc`). Nothing here talks
//! to hardware: each check exists so a missing piece fails with a fix hint
//! instead of failing mid-task with a confusing tool error.

use crate::install;
use firment_core::config::config_path;
use firment_core::{Config, load_auth};
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
    let key = config.api_key_for(provider, name);
    let label = match &key {
        Some(_) if provider.api_key.as_deref().is_some_and(|k| !k.is_empty()) => {
            "configured (inline)".to_string()
        }
        Some(_) if load_auth().contains_key(name) => "configured (auth.json)".to_string(),
        Some(_) => format!(
            "configured via ${}",
            provider.api_key_env.as_deref().unwrap_or_default()
        ),
        None => match provider.api_key_env.as_deref() {
            Some(env_name) if env::var(env_name).is_ok() => {
                format!("MISSING (${env_name} is empty)")
            }
            Some(env_name) => format!("MISSING (${env_name} not set)"),
            None => "MISSING (no api_key or api_key_env)".to_string(),
        },
    };
    (key, label)
}

pub(crate) async fn doctor(config: &Config, path: &Path) -> anyhow::Result<()> {
    println!("config file: {}", path.display());
    if config.providers.is_empty() {
        println!("no providers configured");
        return Ok(());
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
            Ok(response) => println!("  probe {probe_url}: HTTP {}", response.status()),
            Err(e) => {
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
    Ok(())
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
pub(crate) fn doctor_tools(cwd: &Path, tools: &firment_core::config::ToolsConfig) {
    println!("\ntoolchain (optional, only needed for matching project types):");
    for (name, what) in [
        ("pio", "PlatformIO CLI — platformio.ini projects"),
        ("cmake", "CMake — CMakeLists.txt projects"),
        ("make", "GNU make — Makefile projects"),
        ("uv4", "Keil MDK uVision — *.uvprojx projects"),
    ] {
        println!(
            "  {:<8}: {:<24} {}",
            name,
            if which(name).is_some() {
                "found"
            } else {
                "not found"
            },
            what
        );
    }
    match std::process::Command::new("probe-rs")
        .arg("--version")
        .output()
    {
        Ok(o) if o.status.success() => {
            let version = String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            println!("  probe-rs : found {version} — required for flash/run");
        }
        _ => println!(
            "  probe-rs : NOT FOUND — flash/run will fail; install via `cargo install \
             probe-rs-tools` or the probe-rs GitHub releases"
        ),
    }
    let sigrok_bin = tools
        .la
        .as_ref()
        .and_then(|la| la.bin.clone())
        .unwrap_or_else(|| "sigrok-cli".to_string());
    match std::process::Command::new(&sigrok_bin)
        .arg("--version")
        .output()
    {
        Ok(o) if o.status.success() => {
            let version = String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            println!("  {sigrok_bin} : found {version} — required for the la tool");
        }
        _ => println!(
            "  {sigrok_bin} : NOT FOUND — the la tool will fail; install sigrok-cli (Windows: \
             the self-extracting package from sigrok.org/download, then add it to PATH or point \
             [tools.la] bin at sigrok-cli.exe; Linux: your distro's package; macOS: brew)"
        ),
    }

    println!("\nserial ports:");
    let ports = firment_tools::tools::monitor::enumerate_ports();
    println!("  {ports}");

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
