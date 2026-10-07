use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::time::Duration;

/// `models`: list the model ids a configured provider endpoint serves
/// (GET {base_url}/models). Discovery for task-tool delegation — e.g. pick
/// the small SBC model instead of assuming only the configured one exists.
pub struct Models;

/// What an HTTP status that is not a model list usually means, in the words a
/// `firm doctor`/`models` reader can act on.
///
/// Named `hint` rather than `reason` because it is a shape, not a diagnosis: the body may say
/// more. What matters is that the line stops claiming "listed no models" about a request the
/// endpoint refused outright -- the two facts send the reader to different places, a key versus
/// a catalog.
fn status_hint(code: u16) -> &'static str {
    match code {
        401 | 403 => "the endpoint refused the credential (check the provider's api_key / env)",
        404 => "no /models on this base URL (check [providers].base_url)",
        408 => "the request timed out at the endpoint",
        429 => "rate limited; retry later",
        500..=599 => "the endpoint errored; its own logs are the next place to look",
        _ => "the endpoint did not return a model list",
    }
}

#[async_trait]
impl Tool for Models {
    fn name(&self) -> &'static str {
        "models"
    }

    fn description(&self) -> &'static str {
        "List the models served by configured OpenAI-compatible provider endpoints (GET /models). Use before delegating with the task tool's provider/model overrides: it shows which models actually exist on each backend — an SBC ollama may serve several small models, cloud APIs several tiers. Without arguments every configured provider is probed; pass provider to query just one."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "description": "Provider name from config.toml [providers]. Omit to list all."
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        if ctx.providers.is_empty() {
            return Ok(ToolOutput {
                text: "No providers configured. Add one to config.toml ([providers.<name>] with type/base_url/model)."
                    .into(),
            });
        }

        let wanted = args.get("provider").and_then(|p| p.as_str());
        let endpoints: Vec<_> = match wanted {
            Some(name) => {
                let ep = ctx
                    .providers
                    .iter()
                    .find(|e| e.name == name)
                    .ok_or_else(|| {
                        let known = ctx
                            .providers
                            .iter()
                            .map(|e| e.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        ToolError::new(format!(
                            "[InvalidInput] unknown provider '{name}' (configured: {known})"
                        ))
                    })?;
                vec![ep.clone()]
            }
            None => ctx.providers.clone(),
        };

        let client = firment_core::http_builder()
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| ToolError::new(format!("[Http] client: {e}")))?;

        let mut lines = Vec::new();
        for ep in &endpoints {
            let base = ep.base_url.trim_end_matches('/');
            let mut req = client.get(format!("{base}/models"));
            if let Some(key) = &ep.api_key {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) => {
                    // The status has to be read before the body. A 401 answers with a JSON error
                    // object, so `data` is absent, and the old fallthrough printed "reachable
                    // but listed no models" -- which tells a person with the wrong key that their
                    // endpoint holds no models, and sends them looking at the model list instead
                    // of at the credential. The endpoint did answer; what it said was no.
                    let status = resp.status();
                    let ids = resp.json::<Value>().await.ok().and_then(|v| {
                        Some(
                            v.get("data")?
                                .as_array()?
                                .iter()
                                .filter_map(|m| m.get("id")?.as_str().map(String::from))
                                .collect::<Vec<_>>(),
                        )
                    });
                    if !status.is_success() {
                        lines.push(format!(
                            "{0} @ {1}: answered HTTP {status} — {hint}",
                            ep.name,
                            base,
                            hint = status_hint(status.as_u16())
                        ));
                    } else {
                        match ids {
                            Some(ids) if !ids.is_empty() => {
                                lines.push(format!("{} @ {}: {}", ep.name, base, ids.join(", ")));
                            }
                            Some(_) => lines.push(format!(
                                "{0} @ {1}: reachable but listed no models",
                                ep.name, base
                            )),
                            // A 200 whose body is not a model list is a different fact from an
                            // empty list: a proxy page, a redirect to a login form, an API that
                            // answers `/models` with something else entirely.
                            None => lines.push(format!(
                                "{0} @ {1}: answered HTTP {status} with a body that is not a \
                                 model list",
                                ep.name, base
                            )),
                        }
                    }
                }
                Err(e) => lines.push(format!(
                    "{} @ {}: unreachable ({e}) — is the backend running?",
                    ep.name, base
                )),
            }
        }

        Ok(ToolOutput {
            text: lines.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::AutoApprove;
    use std::sync::Arc;

    #[tokio::test]
    async fn unknown_provider_lists_known_names() {
        let mut ctx = ToolContext::with_cwd(std::env::temp_dir());
        ctx.permission = Arc::new(AutoApprove::everything());
        ctx.providers = vec![firment_core::tool::ProviderEndpoint {
            name: "sbc-ollama".into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            api_key: None,
        }];
        let err = Models
            .run(json!({"provider": "nope"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("configured: sbc-ollama"));
    }

    #[tokio::test]
    async fn no_providers_is_a_clean_message() {
        let mut ctx = ToolContext::with_cwd(std::env::temp_dir());
        ctx.permission = Arc::new(AutoApprove::everything());
        let out = Models.run(json!({}), &ctx).await.unwrap();
        assert!(out.text.contains("No providers configured"));
    }

    #[test]
    fn registered_in_all() {
        assert!(crate::tools::all().iter().any(|t| t.name() == "models"));
    }

    #[test]
    fn plan_registry_includes_models() {
        let reg = crate::plan_registry();
        assert!(reg.get("models").is_some());
    }

    #[test]
    fn a_refusal_is_not_reported_as_an_empty_catalog() {
        // The probe read no status: a 401 body parses to no `data` array, and the line printed
        // "reachable but listed no models" -- a sentence that sends a person with a wrong key to
        // look at their model list. The status is now taken before the body, and these are the
        // two facts the reader has to be able to tell apart.
        assert!(
            status_hint(401).contains("credential"),
            "{:?}",
            status_hint(401)
        );
        assert!(status_hint(403).contains("credential"));
        assert!(status_hint(404).contains("base_url"));
        assert!(status_hint(429).contains("rate"));
        assert!(status_hint(503).contains("its own logs"));
        // An unknown code still says something actionable rather than nothing.
        assert!(status_hint(418).contains("model list"));
        // And the phrase the bug produced is unreachable from here: no status that is not a
        // success can be described as having listed no models.
        for code in [400u16, 401, 403, 404, 429, 500, 503] {
            assert!(
                !status_hint(code).contains("listed no models"),
                "HTTP {code} must not be phrased as an empty catalog"
            );
        }
    }
}
