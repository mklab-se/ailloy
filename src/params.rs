//! Static registry of per-capability request parameters.
//!
//! This is the single source of truth for the tunable parameters a node can
//! carry as defaults (e.g. `image.size`, `chat.temperature`). It is pure data
//! plus lookup/validation helpers — no I/O, no feature gate. Consumers:
//! client-side defaults resolution (merging a node's stored defaults into a
//! request) and the interactive config TUI (rendering an editable parameter
//! table per node, filtered by provider and capability).

use crate::config::{Capability, ProviderKind};

/// The shape of a parameter's value space, used to render and validate input.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    /// One of a fixed set of string values.
    Enum(&'static [&'static str]),
    /// An unsigned integer in `[min, max]`.
    UInt { min: u64, max: u64 },
    /// A floating point value in `[min, max]`.
    Float { min: f64, max: f64 },
    /// A `WxH` size string, e.g. `"1024x1024"`.
    Size,
}

/// Definition of a single tunable parameter for a capability.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    /// Dotted key, e.g. `"image.size"`.
    pub key: &'static str,
    /// Capability this parameter applies to.
    pub capability: Capability,
    /// Human-readable label for display in the TUI.
    pub label: &'static str,
    /// The value shape used for validation and input rendering.
    pub kind: ParamKind,
    /// A display hint for the provider default, if any. Not auto-sent —
    /// purely informational (e.g. shown as a placeholder in the TUI).
    pub default: Option<&'static str>,
    /// Providers this parameter applies to. Empty means "all providers that
    /// support this parameter's capability".
    pub providers: &'static [ProviderKind],
}

/// The full static parameter registry.
pub const PARAMS: &[ParamDef] = &[
    ParamDef {
        key: "image.size",
        capability: Capability::Image,
        label: "Size",
        kind: ParamKind::Size,
        default: None,
        providers: &[],
    },
    ParamDef {
        key: "image.quality",
        capability: Capability::Image,
        label: "Quality",
        kind: ParamKind::Enum(&["low", "medium", "high", "auto"]),
        default: Some("auto"),
        providers: &[],
    },
    ParamDef {
        key: "image.format",
        capability: Capability::Image,
        label: "Format",
        kind: ParamKind::Enum(&["png", "jpeg"]),
        default: Some("png"),
        providers: &[
            ProviderKind::OpenAi,
            ProviderKind::AzureOpenAi,
            ProviderKind::MicrosoftFoundry,
        ],
    },
    ParamDef {
        key: "image.compression",
        capability: Capability::Image,
        label: "Compression",
        kind: ParamKind::UInt { min: 0, max: 100 },
        default: None,
        providers: &[
            ProviderKind::OpenAi,
            ProviderKind::AzureOpenAi,
            ProviderKind::MicrosoftFoundry,
        ],
    },
    ParamDef {
        key: "image.background",
        capability: Capability::Image,
        label: "Background",
        kind: ParamKind::Enum(&["transparent", "opaque", "auto"]),
        default: None,
        providers: &[
            ProviderKind::OpenAi,
            ProviderKind::AzureOpenAi,
            ProviderKind::MicrosoftFoundry,
        ],
    },
    ParamDef {
        key: "image.variants",
        capability: Capability::Image,
        label: "Variants",
        kind: ParamKind::UInt { min: 1, max: 10 },
        default: Some("1"),
        providers: &[],
    },
    ParamDef {
        key: "video.size",
        capability: Capability::Video,
        label: "Size",
        kind: ParamKind::Size,
        default: None,
        providers: &[ProviderKind::AzureOpenAi, ProviderKind::MicrosoftFoundry],
    },
    ParamDef {
        key: "video.seconds",
        capability: Capability::Video,
        label: "Seconds",
        kind: ParamKind::UInt { min: 1, max: 20 },
        default: Some("5"),
        providers: &[ProviderKind::AzureOpenAi, ProviderKind::MicrosoftFoundry],
    },
    ParamDef {
        key: "video.variants",
        capability: Capability::Video,
        label: "Variants",
        kind: ParamKind::UInt { min: 1, max: 5 },
        default: Some("1"),
        providers: &[ProviderKind::AzureOpenAi, ProviderKind::MicrosoftFoundry],
    },
    ParamDef {
        key: "chat.temperature",
        capability: Capability::Chat,
        label: "Temperature",
        kind: ParamKind::Float { min: 0.0, max: 2.0 },
        default: None,
        providers: &[],
    },
    ParamDef {
        key: "chat.max_tokens",
        capability: Capability::Chat,
        label: "Max tokens",
        kind: ParamKind::UInt {
            min: 1,
            max: 400_000,
        },
        default: None,
        providers: &[],
    },
    ParamDef {
        key: "embedding.dimensions",
        capability: Capability::Embedding,
        label: "Dimensions",
        kind: ParamKind::UInt {
            min: 1,
            max: 10_000,
        },
        default: None,
        providers: &[],
    },
];

/// Returns the parameter definitions applicable to `provider` for the given
/// `caps`. A definition is kept when its capability is in `caps`, the
/// provider supports that capability, and either the definition's
/// `providers` list is empty (applies to all providers supporting the
/// capability) or it explicitly contains `provider`.
pub fn params_for(provider: &ProviderKind, caps: &[Capability]) -> Vec<&'static ParamDef> {
    PARAMS
        .iter()
        .filter(|def| {
            caps.contains(&def.capability)
                && provider.supports_capability(&def.capability)
                && (def.providers.is_empty() || def.providers.contains(provider))
        })
        .collect()
}

/// Validates `value` against `def`'s kind, returning an actionable error
/// message on failure.
pub fn validate_value(def: &ParamDef, value: &str) -> Result<(), String> {
    match &def.kind {
        ParamKind::Enum(allowed) => {
            if allowed.contains(&value) {
                Ok(())
            } else {
                Err(format!(
                    "invalid value '{value}' for '{}': expected one of {}",
                    def.key,
                    allowed.join(", ")
                ))
            }
        }
        ParamKind::UInt { min, max } => {
            let parsed: u64 = value.parse().map_err(|_| {
                format!(
                    "invalid value '{value}' for '{}': expected an integer between {min} and {max}",
                    def.key
                )
            })?;
            if parsed < *min || parsed > *max {
                Err(format!(
                    "invalid value '{value}' for '{}': expected an integer between {min} and {max}",
                    def.key
                ))
            } else {
                Ok(())
            }
        }
        ParamKind::Float { min, max } => {
            let parsed: f64 = value.parse().map_err(|_| {
                format!(
                    "invalid value '{value}' for '{}': expected a number between {min} and {max}",
                    def.key
                )
            })?;
            if parsed < *min || parsed > *max {
                Err(format!(
                    "invalid value '{value}' for '{}': expected a number between {min} and {max}",
                    def.key
                ))
            } else {
                Ok(())
            }
        }
        ParamKind::Size => {
            let err = || {
                format!(
                    "invalid value '{value}' for '{}': expected a size in 'WxH' format (e.g. '1024x1024') with positive width and height",
                    def.key
                )
            };
            let (w, h) = value.split_once('x').ok_or_else(err)?;
            let width: u32 = w.parse().map_err(|_| err())?;
            let height: u32 = h.parse().map_err(|_| err())?;
            if width == 0 || height == 0 {
                Err(err())
            } else {
                Ok(())
            }
        }
    }
}

/// Looks up a parameter definition by its dotted key (e.g. `"image.size"`).
pub fn lookup(key: &str) -> Option<&'static ParamDef> {
    PARAMS.iter().find(|def| def.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_for_filters_by_capability_and_provider() {
        let defs = params_for(
            &ProviderKind::MicrosoftFoundry,
            &[Capability::Image, Capability::Video],
        );
        let keys: Vec<&str> = defs.iter().map(|d| d.key).collect();
        assert!(keys.contains(&"image.format"));
        assert!(keys.contains(&"video.seconds"));
        assert!(!keys.contains(&"chat.temperature"));
    }

    #[test]
    fn params_for_excludes_provider_restricted_params_not_supported() {
        // Vertex AI supports image but not the OpenAI-family-only image params.
        let defs = params_for(&ProviderKind::VertexAi, &[Capability::Image]);
        let keys: Vec<&str> = defs.iter().map(|d| d.key).collect();
        assert!(!keys.contains(&"image.format"));
        assert!(!keys.contains(&"image.compression"));
        assert!(!keys.contains(&"image.background"));
        // Unrestricted (empty providers) image params still apply to Vertex.
        assert!(keys.contains(&"image.size"));
        assert!(keys.contains(&"image.quality"));
        assert!(keys.contains(&"image.variants"));
    }

    #[test]
    fn params_for_excludes_unsupported_capability() {
        // Anthropic only supports chat.
        let defs = params_for(
            &ProviderKind::Anthropic,
            &[Capability::Chat, Capability::Image, Capability::Video],
        );
        assert!(defs.iter().all(|d| d.capability == Capability::Chat));
        assert!(!defs.is_empty());
    }

    #[test]
    fn params_for_video_restricted_to_azure_and_foundry() {
        let defs = params_for(&ProviderKind::OpenAi, &[Capability::Video]);
        // OpenAI doesn't support video at all, so nothing should be returned.
        assert!(defs.is_empty());
    }

    #[test]
    fn validate_value_enum_member_and_non_member() {
        let def = lookup("image.quality").unwrap();
        assert!(validate_value(def, "high").is_ok());
        let err = validate_value(def, "ultra").unwrap_err();
        assert!(err.contains("low"));
        assert!(err.contains("medium"));
        assert!(err.contains("high"));
        assert!(err.contains("auto"));
    }

    #[test]
    fn validate_value_uint_in_and_out_of_range() {
        let def = lookup("image.compression").unwrap();
        assert!(validate_value(def, "0").is_ok());
        assert!(validate_value(def, "100").is_ok());
        assert!(validate_value(def, "101").is_err());
        assert!(validate_value(def, "-1").is_err());
        assert!(validate_value(def, "notanumber").is_err());
    }

    #[test]
    fn validate_value_float_in_and_out_of_range() {
        let def = lookup("chat.temperature").unwrap();
        assert!(validate_value(def, "0.7").is_ok());
        assert!(validate_value(def, "0").is_ok());
        assert!(validate_value(def, "2").is_ok());
        assert!(validate_value(def, "2.1").is_err());
        assert!(validate_value(def, "-0.1").is_err());
        assert!(validate_value(def, "nope").is_err());
    }

    #[test]
    fn validate_value_size_parses_and_rejects_zero() {
        let def = lookup("image.size").unwrap();
        assert!(validate_value(def, "1024x1024").is_ok());
        assert!(validate_value(def, "1920x1080").is_ok());
        assert!(validate_value(def, "0x1024").is_err());
        assert!(validate_value(def, "1024x0").is_err());
        assert!(validate_value(def, "1024").is_err());
        assert!(validate_value(def, "abcxdef").is_err());
    }

    #[test]
    fn every_default_passes_its_own_validation() {
        for def in PARAMS {
            if let Some(default) = def.default {
                assert!(
                    validate_value(def, default).is_ok(),
                    "default '{default}' for '{}' failed self-validation",
                    def.key
                );
            }
        }
    }

    #[test]
    fn lookup_hit_and_miss() {
        assert!(lookup("image.size").is_some());
        assert_eq!(lookup("chat.temperature").unwrap().key, "chat.temperature");
        assert!(lookup("nonexistent.key").is_none());
    }
}
