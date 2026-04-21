//! Factory function for creating the appropriate [`Summarizer`] implementation
//! from a [`SummarizationConfig`].

use vox_core::config::SummarizationConfig;

use crate::{
    client::OpenAiClient, error::SummarizeError, ollama::OllamaClient, traits::Summarizer,
};

/// Create a [`Summarizer`] from the application configuration.
///
/// # Backend mapping
///
/// | `config.backend` | Implementation |
/// |---|---|
/// | `"ollama"` | [`OllamaClient`] using native `/api/chat` endpoint |
/// | `"openai_compatible"` | [`OpenAiClient`] pointed at `config.api_url` |
/// | `"builtin"` | Returns [`SummarizeError::BackendNotImplemented`] |
/// | anything else | Returns [`SummarizeError::UnknownBackend`] |
///
/// # Errors
///
/// - [`SummarizeError::BackendNotImplemented`] when `backend = "builtin"`.
/// - [`SummarizeError::UnknownBackend`] for unrecognised backend names.
/// - [`SummarizeError::Config`] when required fields (e.g. `api_url`) are
///   empty for the selected backend.
pub fn create_summarizer(
    config: &SummarizationConfig,
) -> Result<Box<dyn Summarizer>, SummarizeError> {
    match config.backend.as_str() {
        "ollama" => {
            let url = if config.ollama_url.is_empty() {
                "http://localhost:11434"
            } else {
                &config.ollama_url
            };
            let model = if config.ollama_model.is_empty() {
                return Err(SummarizeError::Config(
                    "ollama_model must not be empty".to_owned(),
                ));
            } else {
                config.ollama_model.clone()
            };
            tracing::info!(url, model, "creating Ollama summarizer (native API)");
            let client = OllamaClient::new(url, model)?;
            Ok(Box::new(client))
        }

        "openai_compatible" => {
            if config.api_url.is_empty() {
                return Err(SummarizeError::Config(
                    "api_url must be set for openai_compatible backend".to_owned(),
                ));
            }
            if config.api_model.is_empty() {
                return Err(SummarizeError::Config(
                    "api_model must be set for openai_compatible backend".to_owned(),
                ));
            }
            let api_key = if config.api_key.is_empty() {
                None
            } else {
                Some(config.api_key.clone())
            };
            tracing::info!(
                url = %config.api_url,
                model = %config.api_model,
                "creating OpenAI-compatible summarizer"
            );
            let client = OpenAiClient::new(
                &config.api_url,
                api_key,
                config.api_model.clone(),
                "openai_compatible",
            )?;
            Ok(Box::new(client))
        }

        "builtin" => Err(SummarizeError::BackendNotImplemented("builtin".to_owned())),

        other => Err(SummarizeError::UnknownBackend(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_core::config::SummarizationConfig;

    fn ollama_config() -> SummarizationConfig {
        SummarizationConfig {
            backend: "ollama".to_owned(),
            ollama_url: "http://localhost:11434".to_owned(),
            ollama_model: "qwen2.5:1.5b".to_owned(),
            ..SummarizationConfig::default()
        }
    }

    fn openai_config() -> SummarizationConfig {
        SummarizationConfig {
            backend: "openai_compatible".to_owned(),
            api_url: "https://api.openai.com".to_owned(),
            api_key: "sk-test".to_owned(),
            api_model: "gpt-4o".to_owned(),
            ..SummarizationConfig::default()
        }
    }

    #[test]
    fn test_create_ollama_summarizer() {
        let config = ollama_config();
        let result = create_summarizer(&config);
        assert!(result.is_ok(), "expected Ok but got an Err");
    }

    #[test]
    fn test_create_openai_compatible_summarizer() {
        let config = openai_config();
        let result = create_summarizer(&config);
        assert!(result.is_ok(), "expected Ok but got an Err");
    }

    #[test]
    fn test_openai_compatible_missing_url_returns_config_error() {
        let config = SummarizationConfig {
            backend: "openai_compatible".to_owned(),
            api_url: String::new(),
            api_model: "gpt-4o".to_owned(),
            ..SummarizationConfig::default()
        };
        assert!(matches!(
            create_summarizer(&config),
            Err(SummarizeError::Config(_))
        ));
    }

    #[test]
    fn test_openai_compatible_missing_model_returns_config_error() {
        let config = SummarizationConfig {
            backend: "openai_compatible".to_owned(),
            api_url: "https://api.openai.com".to_owned(),
            api_model: String::new(),
            ..SummarizationConfig::default()
        };
        assert!(matches!(
            create_summarizer(&config),
            Err(SummarizeError::Config(_))
        ));
    }

    #[test]
    fn test_builtin_returns_not_implemented() {
        let config = SummarizationConfig {
            backend: "builtin".to_owned(),
            ..SummarizationConfig::default()
        };
        assert!(matches!(
            create_summarizer(&config),
            Err(SummarizeError::BackendNotImplemented(_))
        ));
    }

    #[test]
    fn test_unknown_backend_returns_error() {
        let config = SummarizationConfig {
            backend: "anthropic".to_owned(),
            ..SummarizationConfig::default()
        };
        assert!(matches!(
            create_summarizer(&config),
            Err(SummarizeError::UnknownBackend(_))
        ));
    }

    #[test]
    fn test_ollama_missing_model_returns_config_error() {
        let config = SummarizationConfig {
            backend: "ollama".to_owned(),
            ollama_url: "http://localhost:11434".to_owned(),
            ollama_model: String::new(),
            ..SummarizationConfig::default()
        };
        assert!(matches!(
            create_summarizer(&config),
            Err(SummarizeError::Config(_))
        ));
    }

    #[test]
    fn test_openai_compatible_empty_api_key_is_allowed() {
        // An empty api_key is valid — treated as unauthenticated (e.g. local server).
        let config = SummarizationConfig {
            backend: "openai_compatible".to_owned(),
            api_url: "http://localhost:8080".to_owned(),
            api_model: "local-model".to_owned(),
            api_key: String::new(),
            ..SummarizationConfig::default()
        };
        assert!(create_summarizer(&config).is_ok());
    }
}
