//! Factory for building enabled [`ExportTarget`]s from an [`ExportConfig`].

use vox_core::config::ExportConfig;

use crate::{affine::AffineTarget, error::ExportError, traits::ExportTarget};

/// Construct one boxed [`ExportTarget`] per enabled entry in `config`.
///
/// Targets that fail to construct (e.g. missing required fields) are logged
/// and skipped so that one broken config section does not disable the entire
/// export subsystem.
///
/// # Errors
///
/// This function itself cannot fail — individual target construction errors
/// are logged and swallowed. Callers get whichever targets were built
/// successfully.
#[must_use]
pub fn build_targets(config: &ExportConfig) -> Vec<Box<dyn ExportTarget>> {
    let mut out: Vec<Box<dyn ExportTarget>> = Vec::new();

    if config.affine.enabled {
        match AffineTarget::from_config(&config.affine) {
            Ok(target) => out.push(Box::new(target)),
            Err(e) => {
                tracing::warn!(error = %e, "affine export target disabled due to config error");
            }
        }
    }

    out
}

/// Build a single [`ExportTarget`] by id.
///
/// Used by the GUI when the user picks a specific target from the Send-to
/// modal without re-enumerating the full list.
///
/// # Errors
///
/// - [`ExportError::UnknownTarget`] when `id` is not a known target.
/// - [`ExportError::NotConfigured`] when the target exists but its config
///   section is disabled.
/// - [`ExportError::Config`] when required fields are missing.
pub fn build_target_by_id(
    id: &str,
    config: &ExportConfig,
) -> Result<Box<dyn ExportTarget>, ExportError> {
    match id {
        "affine" => {
            if !config.affine.enabled {
                return Err(ExportError::NotConfigured("affine".to_owned()));
            }
            Ok(Box::new(AffineTarget::from_config(&config.affine)?))
        }
        other => Err(ExportError::UnknownTarget(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_core::config::{AffineExportConfig, ExportConfig};

    fn disabled_config() -> ExportConfig {
        ExportConfig::default()
    }

    fn enabled_config() -> ExportConfig {
        ExportConfig {
            affine: AffineExportConfig {
                enabled: true,
                base_url: "https://app.affine.pro".to_owned(),
                api_token: "ut_test".to_owned(),
                email: String::new(),
                password: String::new(),
                default_workspace_id: String::new(),
                default_parent_id: String::new(),
            },
        }
    }

    #[test]
    fn build_targets_empty_when_affine_disabled() {
        let targets = build_targets(&disabled_config());
        assert!(targets.is_empty());
    }

    #[test]
    fn build_targets_includes_affine_when_enabled() {
        let targets = build_targets(&enabled_config());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id(), "affine");
    }

    #[test]
    fn build_target_by_id_unknown_returns_error() {
        let config = enabled_config();
        assert!(matches!(
            build_target_by_id("notion", &config),
            Err(ExportError::UnknownTarget(_))
        ));
    }

    #[test]
    fn build_target_by_id_disabled_returns_not_configured() {
        let config = disabled_config();
        assert!(matches!(
            build_target_by_id("affine", &config),
            Err(ExportError::NotConfigured(_))
        ));
    }

    #[test]
    fn build_target_by_id_missing_auth_returns_config_error() {
        let config = ExportConfig {
            affine: AffineExportConfig {
                enabled: true,
                base_url: "https://app.affine.pro".to_owned(),
                ..AffineExportConfig::default()
            },
        };
        assert!(matches!(
            build_target_by_id("affine", &config),
            Err(ExportError::Config(_))
        ));
    }
}
