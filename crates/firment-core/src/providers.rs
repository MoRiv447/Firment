//! Built-in provider presets ("the catalog").
//!
//! A neutral, alphabetical index of common LLM endpoints so `firm config`
//! can offer one-key setup. The catalog RECOMMENDS nothing: entries carry no
//! rank or badge, nothing is set as default, and the list is not a judgment
//! on any vendor. Users pick what fits their network and budget; model names
//! are the vendor's common ones at build time and can be edited in
//! config.toml at any time.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProviderPreset {
    /// Config key under `[providers.<name>]`.
    pub name: &'static str,
    /// Human-friendly name shown in `firm config`.
    pub display: &'static str,
    /// "openai" (chat/completions) or "anthropic" (messages).
    pub r#type: &'static str,
    /// Endpoint base URL (trailing slash trimmed).
    pub base_url: &'static str,
    /// The environment variable the preset conventionally expects for the
    /// key. `None` = no key needed (local servers).
    pub api_key_env: Option<&'static str>,
    /// Models the vendor commonly serves; the first is `firm config`'s pick.
    pub models: &'static [&'static str],
    /// One-line note for the config screen (region / proxy / key source).
    pub note: &'static str,
}

pub const CATALOG: &[ProviderPreset] = &[
    ProviderPreset {
        name: "anthropic",
        display: "Anthropic",
        r#type: "anthropic",
        base_url: "https://api.anthropic.com",
        api_key_env: Some("ANTHROPIC_API_KEY"),
        models: &["claude-sonnet-4-5", "claude-opus-5"],
        note: "official API; international",
    },
    ProviderPreset {
        name: "deepseek",
        display: "DeepSeek",
        r#type: "openai",
        base_url: "https://api.deepseek.com/v1",
        api_key_env: Some("DEEPSEEK_API_KEY"),
        models: &["deepseek-v4-flash", "deepseek-v3.1"],
        note: "direct from CN, no proxy needed",
    },
    ProviderPreset {
        name: "gemini",
        display: "Google Gemini",
        r#type: "openai",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: Some("GEMINI_API_KEY"),
        models: &["gemini-2.5-pro", "gemini-2.5-flash"],
        note: "international; may need a proxy",
    },
    ProviderPreset {
        name: "glm",
        display: "Zhipu GLM",
        r#type: "openai",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        api_key_env: Some("ZHIPUAI_API_KEY"),
        models: &["glm-4-plus", "glm-4-flash"],
        note: "direct from CN",
    },
    ProviderPreset {
        name: "lmstudio",
        display: "LM Studio",
        r#type: "openai",
        base_url: "http://localhost:1234/v1",
        api_key_env: None,
        models: &["local-model"],
        note: "local server, no key",
    },
    ProviderPreset {
        name: "moonshot",
        display: "Moonshot (Kimi)",
        r#type: "openai",
        base_url: "https://api.moonshot.cn/v1",
        api_key_env: Some("MOONSHOT_API_KEY"),
        models: &["moonshot-v1-32k"],
        note: "direct from CN",
    },
    ProviderPreset {
        name: "ollama",
        display: "Ollama",
        r#type: "openai",
        base_url: "http://localhost:11434/v1",
        api_key_env: None,
        models: &["llama3.2", "qwen3"],
        note: "local server, no key",
    },
    ProviderPreset {
        name: "openai",
        display: "OpenAI",
        r#type: "openai",
        base_url: "https://api.openai.com/v1",
        api_key_env: Some("OPENAI_API_KEY"),
        models: &["gpt-5", "gpt-5-mini"],
        note: "official API; international",
    },
    ProviderPreset {
        name: "openrouter",
        display: "OpenRouter",
        r#type: "openai",
        base_url: "https://openrouter.ai/api/v1",
        api_key_env: Some("OPENROUTER_API_KEY"),
        models: &["deepseek/deepseek-v4-pro", "anthropic/claude-sonnet-4.5"],
        note: "aggregator; international",
    },
    ProviderPreset {
        name: "orcarouter",
        display: "OrcaRouter",
        r#type: "openai",
        base_url: "https://api.orcarouter.ai/v1",
        api_key_env: Some("ORCA_KEY"),
        models: &["orcarouter/auto", "deepseek/deepseek-v4-pro"],
        note: "aggregator; prefer a FIXED model for coding agents — auto routing is not audit-friendly",
    },
    ProviderPreset {
        name: "qwen",
        display: "Alibaba Qwen",
        r#type: "openai",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        api_key_env: Some("DASHSCOPE_API_KEY"),
        models: &["qwen-max", "qwen-plus"],
        note: "direct from CN",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_entries_are_well_formed() {
        let mut names = HashSet::new();
        for p in CATALOG {
            assert!(!p.name.is_empty(), "empty name");
            assert!(names.insert(p.name), "duplicate name: {}", p.name);
            assert!(!p.display.is_empty(), "empty display for {}", p.name);
            assert!(
                matches!(p.r#type, "openai" | "anthropic"),
                "{}: bad type {}",
                p.name,
                p.r#type
            );
            assert!(
                p.base_url.starts_with("https://") || p.base_url.starts_with("http://"),
                "{}: bad base_url {}",
                p.name,
                p.base_url
            );
            assert!(!p.models.is_empty(), "{}: no models", p.name);
            assert!(!p.note.is_empty(), "{}: no note", p.name);
        }
    }

    #[test]
    fn names_are_valid_toml_keys() {
        // Provider names become `[providers.<name>]` sections — they must not
        // carry dots or quotes that would break TOML.
        for p in CATALOG {
            assert!(
                !p.name.contains(['.', '"', '\'']),
                "{}: name would break a TOML section header",
                p.name
            );
        }
    }
}
