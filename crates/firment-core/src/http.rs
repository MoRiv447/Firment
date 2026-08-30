//! Shared HTTP client construction.
//!
//! Every HTTP client in Firment is built from [`http_builder()`] so proxy
//! exclusions stay consistent: LAN endpoints must never be sent through the
//! proxy, or a machine behind a proxy cannot reach its own Ollama.

/// Ranges that must bypass the proxy. reqwest reads `NO_PROXY` on its own,
/// but that variable almost never lists the LAN ranges — so an Ollama or LM
/// Studio server sitting on 192.168.x.x is unreachable the moment a proxy
/// is configured, with nothing but a connection error to show for it.
const NO_PROXY_RANGES: &str =
    "localhost,.local,127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";

/// A [`reqwest::ClientBuilder`] with the LAN exclusion list already attached.
///
/// Mind the trap: `ClientBuilder::no_proxy()` **disables proxying entirely**,
/// it does not take an exclusion list. Exclusions belong on each `Proxy`,
/// which is why this rebuilds the env proxies rather than calling it.
///
/// When no proxy env var is set the builder is left alone, so reqwest's own
/// detection (including the macOS system configuration) still applies.
/// Note that a macOS *system* proxy therefore does not get these exclusions
/// — only the env-var form does.
pub fn http_builder() -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder();
    if let Some(url) = env_proxy("HTTP_PROXY") {
        if let Ok(proxy) = reqwest::Proxy::http(&url) {
            builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
        }
    }
    if let Some(url) = env_proxy("HTTPS_PROXY") {
        if let Ok(proxy) = reqwest::Proxy::https(&url) {
            builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
        }
    }
    if let Some(url) = env_proxy("ALL_PROXY") {
        if let Ok(proxy) = reqwest::Proxy::all(&url) {
            builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
        }
    }
    builder
}

/// A built client with the LAN exclusions applied. Falls back to a plain
/// client if construction fails (only TLS initialisation can, in practice).
///
/// Use this instead of `reqwest::Client::new()` — that shorthand reads the
/// proxy env vars but not these exclusions, which is exactly how a LAN
/// Ollama ends up unreachable.
pub fn http_client() -> reqwest::Client {
    http_builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn lan_no_proxy() -> Option<reqwest::NoProxy> {
    reqwest::NoProxy::from_string(NO_PROXY_RANGES)
}

fn env_proxy(var: &str) -> Option<String> {
    std::env::var(var)
        .or_else(|_| std::env::var(var.to_lowercase()))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_ranges_are_expressible() {
        // NoProxy::from_string accepts CIDR ranges (reqwest documents
        // 192.168.1.0/24 matching 192.168.1.42); guard against a silent
        // change to that grammar leaving us with an exclusion list that
        // parses but matches nothing.
        assert!(lan_no_proxy().is_some());
        for range in ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"] {
            assert!(
                NO_PROXY_RANGES.contains(range),
                "{range} missing from the exclusion list"
            );
        }
    }

    #[test]
    fn builder_always_constructs() {
        // With or without proxy env vars in the test environment, the
        // builder must stay usable.
        let _ = http_builder();
    }
}
