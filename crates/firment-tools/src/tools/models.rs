use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::time::Duration;

/// `models`: list the model ids a configured provider endpoint serves
/// (GET {base_url}/models). Discovery for task-tool delegation — e.g. pick
/// the small SBC model instead of assuming only the configured one exists.
pub struct Models;

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
                    let ids = resp.json::<Value>().await.ok().and_then(|v| {
                        Some(
                            v.get("data")?
                                .as_array()?
                                .iter()
                                .filter_map(|m| m.get("id")?.as_str().map(String::from))
                                .collect::<Vec<_>>(),
                        )
                    });
                    match ids {
                        Some(ids) if !ids.is_empty() => {
                            lines.push(format!("{} @ {}: {}", ep.name, base, ids.join(", ")));
                        }
                        _ => lines.push(format!(
                            "{0} @ {1}: reachable but listed no models",
                            ep.name, base
                        )),
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
}
