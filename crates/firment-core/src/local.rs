//! Local model endpoints (plan §5, item 5).
//!
//! Three jobs, and the plan's own negative-optimisation review constrains all of them
//! (§16.2-6): **probe only from `firm config` and `doctor`**, with a **200 ms** timeout and
//! a **cache**. The reason is in the constraint itself — a probe on every startup makes
//! startup slower for everyone, including the people who have no local model at all. So
//! nothing here runs unless a human asked, and what it learns is remembered.
//!
//! The three jobs:
//!
//! * **Probe** the ports the usual local servers listen on. Short timeout, no retries, no
//!   discovery tricks: `127.0.0.1:11434` is where Ollama is or it is not.
//! * **Cache** the answer, because a person running `firm doctor` twice should not pay for
//!   it twice. The cache expires, because the answer changes when someone starts a server.
//! * **Recommend** a model for the memory available — and say nothing when it does not know
//!   the memory, which is the difference between a suggestion and a guess.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{Config, config_dir};

/// How long a probe may take. The plan's number (§16.2-6), and short enough that a
/// `firm doctor` on a machine with no local server does not feel like a network call.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// How long an answer stays good.
///
/// Long enough that the second `firm doctor` of a session is free, short enough that
/// starting Ollama and asking again works without knowing about a cache.
pub const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// The local servers worth asking, in the order they are usually run.
pub const KNOWN_ENDPOINTS: [(&str, &str, u16); 3] = [
    ("ollama", "Ollama", 11434),
    ("llama.cpp", "llama.cpp's server", 8080),
    ("lm-studio", "LM Studio", 1234),
];

/// One reachable local server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEndpoint {
    /// Which known server this is (`ollama`, `llama.cpp`, `lm-studio`).
    pub kind: String,
    pub base_url: String,
    /// What it says it has. Empty when the server answered but listed nothing — which is a
    /// different fact from "there is no server", and the report keeps them apart.
    pub models: Vec<String>,
}

impl LocalEndpoint {
    /// The provider block this endpoint would need, ready to paste into a config.
    pub fn as_provider(&self) -> crate::config::ProviderConfig {
        crate::config::ProviderConfig {
            r#type: "openai".to_string(),
            base_url: Some(self.base_url.clone()),
            api_key_env: None,
            // Local servers do not check keys, but several refuse a request that carries
            // none at all; a placeholder is what their own docs suggest.
            api_key: Some("local".to_string()),
            model: self
                .models
                .first()
                .cloned()
                .unwrap_or_else(|| "unset".to_string()),
            max_tokens: None,
            temperature: None,
        }
    }
}

/// What was probed, and when.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeCache {
    /// Seconds since the Unix epoch. A number rather than a formatted time so the entry
    /// does not depend on a locale or a timezone to be read back.
    pub at: u64,
    pub endpoints: Vec<LocalEndpoint>,
}

/// Where the cache lives: beside the config, not inside it.
///
/// Not in `config.toml`: that file is the user's, hand-edited and shared in bug reports,
/// and a probe timestamp appearing in it would be noise at best.
pub fn cache_path() -> PathBuf {
    config_dir().join("local-probe.json")
}

/// Whether a cache entry from `at` is still good at `now`.
pub fn cache_is_fresh(at: u64, now: u64) -> bool {
    now.saturating_sub(at) <= CACHE_TTL.as_secs()
}

/// Read the cached probe, if there is a fresh one.
pub fn cached(now: u64) -> Option<Vec<LocalEndpoint>> {
    cached_at(&cache_path(), now)
}

/// `cached` with the path passed in, so a test does not touch the machine's real cache.
pub fn cached_at(path: &Path, now: u64) -> Option<Vec<LocalEndpoint>> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: ProbeCache = serde_json::from_str(&text).ok()?;
    cache_is_fresh(cache.at, now).then_some(cache.endpoints)
}

/// Remember a probe result. A failure to write is not an error worth reporting: the cache
/// exists to save time, and a machine where it cannot be written still works.
pub fn store(endpoints: &[LocalEndpoint], now: u64) {
    store_at(&cache_path(), endpoints, now);
}

pub fn store_at(path: &Path, endpoints: &[LocalEndpoint], now: u64) {
    let cache = ProbeCache {
        at: now,
        endpoints: endpoints.to_vec(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(path, text);
    }
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A model a local server has, paired with what it would cost.
///
/// Not `Eq`: the cost is an `f32`, and claiming an equality relation over floats to
/// satisfy a derive is the kind of thing this codebase does not do.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFit {
    pub name: String,
    /// Rough memory requirement in GB, or `None` when nothing is known about it.
    pub needs_gb: Option<f32>,
}

/// What is known about the memory a model needs, by name fragment.
///
/// A table rather than a calculation, because a formula over parameter count would be
/// wrong for every quantisation, and wrong in the direction that matters: it would suggest
/// a model that does not fit. Fragments are matched case-insensitively and take the first
/// hit, so a specific entry (`q5_k_m`) can sit above a general one.
///
/// The numbers are the ones that matter to a reader with 8 GB: a 0.8 B model is ~1 GB, a
/// 9 B at Q5 is ~6.5 GB, a 7 B at Q4 is ~4.5 GB. Anything unknown stays unknown.
pub const KNOWN_SIZES: [(&str, f32); 8] = [
    ("0.5b", 0.7),
    ("0.8b", 1.0),
    ("1.5b", 1.6),
    ("3b", 2.6),
    ("7b", 4.5),
    ("8b", 5.0),
    ("9b", 6.5),
    ("14b", 9.5),
];

/// What each model would cost on this machine.
pub fn fits(models: &[String]) -> Vec<ModelFit> {
    models
        .iter()
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            let needs_gb = KNOWN_SIZES
                .iter()
                .find(|(fragment, _)| lower.contains(fragment))
                .map(|(_, gb)| *gb);
            ModelFit {
                name: name.clone(),
                needs_gb,
            }
        })
        .collect()
}

/// Which of these models fit in `vram_gb`, best first (largest that still fits).
///
/// `None` for the memory means **no recommendation**: listing models with a "fits" verdict
/// computed from a number nobody supplied would be the guess this module refuses to make.
/// Unknown model sizes are left out of the ranking for the same reason.
pub fn recommend(models: &[String], vram_gb: Option<f32>) -> Vec<ModelFit> {
    let Some(vram) = vram_gb else {
        return Vec::new();
    };
    // Leave room for the KV cache and the desktop that is already using the card. Numbers
    // people quote for "will it fit" assume a headless machine; this one usually is not.
    let budget = vram * 0.85;
    let mut candidates: Vec<ModelFit> = fits(models)
        .into_iter()
        .filter(|model| model.needs_gb.is_some_and(|needs| needs <= budget))
        .collect();
    candidates.sort_by(|a, b| {
        b.needs_gb
            .partial_cmp(&a.needs_gb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

/// Whether a base URL points at this machine or the network it sits on.
///
/// "Local model" means "not a cloud account", and for a lot of people that includes the
/// box in the corner running Ollama — this project's own `[providers]` has one on
/// `192.168.1.8`. So the private ranges count, and the check is a string test rather than a
/// DNS lookup: resolving a name to decide whether to probe it would be a network call
/// deciding whether to make a network call.
pub fn is_private_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let host = lower
        .split("//")
        .nth(1)
        .unwrap_or(&lower)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    if host == "localhost" || host == "::1" || host == "[::1]" {
        return true;
    }
    let octets: Vec<u8> = host
        .split('.')
        .filter_map(|part| part.parse::<u8>().ok())
        .collect();
    match octets.as_slice() {
        [127, ..] => true,
        [10, ..] => true,
        [192, 168, ..] => true,
        [172, second, ..] => (16..=31).contains(second),
        _ => false,
    }
}

/// Probe every known endpoint plus any extra base URLs, concurrently, with
/// [`PROBE_TIMEOUT`].
///
/// Called by `firm config` and `firm doctor` and by nothing else (§16.2-6). Each probe is
/// one HTTP GET with a 200 ms budget: a server that is not there answers "connection
/// refused" immediately, and a server that is there but wedged is abandoned rather than
/// waited on.
///
/// `extra` is how a machine whose Ollama lives on the LAN gets found: the caller passes the
/// base URLs it already knows about that are not cloud (see [`is_private_url`]), rather
/// than this module inventing a discovery protocol.
pub async fn probe_with(extra: &[String]) -> Vec<LocalEndpoint> {
    let mut targets: Vec<(String, String)> = KNOWN_ENDPOINTS
        .iter()
        .map(|(kind, _, port)| (kind.to_string(), format!("http://127.0.0.1:{port}/v1")))
        .collect();
    for base in extra {
        let base = base.trim_end_matches('/');
        if !is_private_url(base) || targets.iter().any(|(_, known)| known == base) {
            continue;
        }
        targets.push(("configured".to_string(), base.to_string()));
    }

    let tasks = targets.into_iter().map(|(kind, base)| async move {
        let models = list_models(&base).await?;
        Some(LocalEndpoint {
            kind,
            base_url: base,
            models,
        })
    });
    let results = futures::future::join_all(tasks).await;
    results.into_iter().flatten().collect()
}

/// [`probe_with`] with nothing extra.
pub async fn probe() -> Vec<LocalEndpoint> {
    probe_with(&[]).await
}

/// `GET /models` on a local server and read the ids out of it.
///
/// The OpenAI-compatible shape (`{"data":[{"id":"…"}]}`) is what all three known servers
/// speak at this path, and it is also what FIRMENT's own provider client speaks, so the
/// endpoint stays honest about being an OpenAI-compatible base URL.
async fn list_models(base: &str) -> Option<Vec<String>> {
    let client = crate::http_builder().timeout(PROBE_TIMEOUT).build().ok()?;
    let response = client.get(format!("{base}/models")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    let models: Vec<String> = body["data"]
        .as_array()?
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(|id| id.to_string()))
        .collect();
    Some(models)
}

/// The provider block name a local endpoint gets, avoiding a collision with an existing one.
///
/// `ollama` is free until someone has one; then it is `ollama-2`, and the caller does not
/// have to think about it. Deterministic rather than timestamped so running the same switch
/// twice does not create two entries.
pub fn provider_name(kind: &str, config: &Config) -> String {
    if !config.providers.contains_key(kind) {
        return kind.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{kind}-{index}");
        if !config.providers.contains_key(&candidate) {
            return candidate;
        }
        index += 1;
        if index > 50 {
            // Fifty collisions is not a naming problem, it is a sign the caller is looping.
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_cache_is_used_and_a_stale_one_is_not() {
        let now = 1_000_000;
        assert!(cache_is_fresh(now, now));
        assert!(cache_is_fresh(now - CACHE_TTL.as_secs(), now));
        // One second past the window: the answer may have changed, so it is re-probed.
        assert!(!cache_is_fresh(now - CACHE_TTL.as_secs() - 1, now));
        // A clock that jumped backwards must not make an entry immortal.
        assert!(cache_is_fresh(now + 10_000, now));
    }

    #[test]
    fn a_cached_probe_round_trips_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local-probe.json");
        let endpoints = vec![LocalEndpoint {
            kind: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            models: vec!["qwen3.5:0.8b".to_string()],
        }];
        store_at(&path, &endpoints, 1000);

        assert_eq!(cached_at(&path, 1000 + 60), Some(endpoints.clone()));
        assert_eq!(cached_at(&path, 1000 + CACHE_TTL.as_secs() + 1), None);
        // A file that is not a probe result is not a probe result.
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(cached_at(&path, 1000), None);
    }

    #[test]
    fn the_sizes_it_knows_are_attached_and_the_rest_are_left_unknown() {
        let models = vec![
            "qwen3.5:0.8b".to_string(),
            "ornith-1.5-9b-q5_k_m".to_string(),
            "some-company/mystery-42b".to_string(),
        ];
        let fitted = fits(&models);
        assert_eq!(fitted[0].needs_gb, Some(1.0));
        assert_eq!(fitted[1].needs_gb, Some(6.5));
        assert_eq!(fitted[2].needs_gb, None, "an unknown size stays unknown");
    }

    #[test]
    fn a_recommendation_needs_to_know_the_memory() {
        let models = vec![
            "qwen3.5:0.8b".to_string(),
            "ornith-1.5-9b-q5_k_m".to_string(),
        ];

        // No memory known: no recommendation, rather than one computed from a guess.
        assert!(recommend(&models, None).is_empty());

        // 8 GB of card, minus the headroom for the KV cache and the desktop: the 9 B at
        // ~6.5 GB fits the budget, the 0.8 B fits easily, and the larger one is offered
        // first because it is the one worth running.
        let fits_8 = recommend(&models, Some(8.0));
        assert_eq!(
            fits_8.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["ornith-1.5-9b-q5_k_m", "qwen3.5:0.8b"]
        );

        // 4 GB: the 9 B is out, and the 0.8 B is all that is left.
        let fits_4 = recommend(&models, Some(4.0));
        assert_eq!(fits_4.len(), 1);
        assert_eq!(fits_4[0].name, "qwen3.5:0.8b");
    }

    #[test]
    fn a_local_endpoint_becomes_a_provider_block_that_can_be_used_as_is() {
        let endpoint = LocalEndpoint {
            kind: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            models: vec!["qwen3.5:0.8b".to_string()],
        };
        let provider = endpoint.as_provider();
        assert_eq!(
            provider.base_url.as_deref(),
            Some(endpoint.base_url.as_str())
        );
        assert_eq!(provider.model, "qwen3.5:0.8b");
        // Several local servers reject a request that carries no auth header at all, which
        // is why the placeholder exists.
        assert!(provider.api_key.is_some());
        assert!(provider.api_key_env.is_none());
    }

    #[test]
    fn the_provider_name_avoids_a_collision_without_a_timestamp() {
        let mut config = Config::default_config();
        assert_eq!(provider_name("ollama", &config), "ollama");

        config.providers.insert(
            "ollama".to_string(),
            Config::default_config()
                .providers
                .values()
                .next()
                .unwrap()
                .clone(),
        );
        assert_eq!(provider_name("ollama", &config), "ollama-2");
    }

    #[test]
    fn a_local_model_can_live_on_the_lan() {
        // This project's own config has Ollama on 192.168.1.8, so "local" has to mean
        // "not a cloud account" rather than "on this motherboard".
        for url in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:11434/v1",
            "http://192.168.1.8:11434/v1",
            "http://10.0.0.5:8080/v1",
            "http://172.16.4.4:1234/v1",
            "http://172.31.255.1:1234/v1",
        ] {
            assert!(is_private_url(url), "{url} should count as local");
        }
        for url in [
            "https://api.deepseek.com/v1",
            "https://open.bigmodel.cn/api/paas/v4",
            "http://172.32.0.1:1234/v1",
            "http://11.0.0.1:11434/v1",
        ] {
            assert!(!is_private_url(url), "{url} is not a local endpoint");
        }
    }

    #[tokio::test]
    async fn probing_a_machine_with_nothing_listening_returns_nothing() {
        // This is the honest offline case, and it is the one that has to be fast: three
        // refused connections, no timeout wait, and no error to report as a failure.
        let endpoints = probe().await;
        assert!(
            endpoints.iter().all(|e| !e.models.is_empty()),
            "a server that answers with no models is not an endpoint: {endpoints:?}"
        );
    }
}
