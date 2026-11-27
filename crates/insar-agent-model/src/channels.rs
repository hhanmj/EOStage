//! Local metadata for user-owned model channels. Secrets belong exclusively to
//! the native credential service; neither the file nor any response carries them.
use crate::{transport::ModelRoute, Protocol, ReasoningMode};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// One writer at a time for the channel files of this process: a run that
/// remembers a refused switch must not overwrite a channel the user saved
/// meanwhile (each write reads the list first).
static WRITE: Mutex<()> = Mutex::new(());

fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Credential-store account keys reserved for non-LLM subsystems that share
/// `AGENT_MODEL_KEYRING_SERVICE` (specifically Google Earth Engine `"gee"`,
/// `"gee-credentials"`, `"gee-*"`, and `"gee_*"`). Rejecting these at channel
/// validation prevents LLM channel operations from reading, overwriting, or
/// deleting GEE credentials.
pub fn is_reserved_channel_id(id: &str) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    lower == "gee"
        || lower == "gee-credentials"
        || lower.starts_with("gee-")
        || lower.starts_with("gee_")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    pub id: String,
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    /// Written only when not `off`, so a channel file without a reasoning
    /// choice still loads in builds before contract 3.31.0 (which refuse
    /// unknown fields).
    #[serde(default, skip_serializing_if = "ReasoningMode::is_off")]
    pub reasoning_mode: ReasoningMode,
    /// The provider refused the reasoning switch for these settings and the
    /// request resent without it succeeded (W3o, L4 r9 P2-1); set only by
    /// [`ChannelStore::remember_reasoning_switch_unsupported`], kept by a
    /// save with the same protocol, base URL and model, cleared by any other
    /// save. Written only when true, so older builds still load the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_switch_unsupported: bool,
    /// The models offered under this channel; empty means just `model`
    /// (channels saved before the list existed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ChannelModel>,
}

/// One model of a channel, as the settings page lists it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// "text", "image", "video", and/or "pdf".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_web_search: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_mid_turn_system: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_param_mapping: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_free: Option<bool>,
}

/// A thinking level the user can pick for a model. `value` is what the
/// request carries (`reasoning_effort`, or a thinking token budget for
/// Anthropic); none sends no thinking field. The level named `off` uses the
/// known per-family switch instead.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReasoningLevel {
    pub level: String,
    #[serde(default)]
    pub value: Option<String>,
}

fn parse_protocol(protocol: &str) -> Option<Protocol> {
    match protocol {
        "chat_completions" => Some(Protocol::ChatCompletions),
        "responses" => Some(Protocol::Responses),
        "anthropic_messages" => Some(Protocol::AnthropicMessages),
        "google_gemini" => Some(Protocol::GoogleGemini),
        _ => None,
    }
}

impl ChannelConfig {
    /// The channel's models; a channel without a list offers its one model
    /// when non-empty.
    pub fn effective_models(&self) -> Vec<ChannelModel> {
        if self.models.is_empty() {
            if self.model.trim().is_empty() {
                Vec::new()
            } else {
                vec![ChannelModel {
                    id: self.model.clone(),
                    ..ChannelModel::default()
                }]
            }
        } else {
            self.models.clone()
        }
    }

    fn valid_models(&self) -> bool {
        let short = |text: &str, max: usize| {
            !text.trim().is_empty() && text.len() <= max && !text.chars().any(char::is_control)
        };
        self.models.len() <= 128
            && self.models.iter().all(|model| {
                short(&model.id, 256)
                    && model.name.as_deref().is_none_or(|name| short(name, 128))
                    && model
                        .protocol
                        .as_deref()
                        .is_none_or(|proto| parse_protocol(proto).is_some())
                    && model
                        .input_types
                        .iter()
                        .all(|kind| matches!(kind.as_str(), "text" | "image" | "video" | "pdf"))
                    && model.reasoning_levels.len() <= 8
                    && model.reasoning_levels.iter().all(|level| {
                        short(&level.level, 32)
                            && level.value.as_deref().is_none_or(|value| short(value, 64))
                    })
                    && model
                        .reasoning_param_mapping
                        .as_deref()
                        .is_none_or(|mapping| {
                            mapping.len() <= 64 && !mapping.chars().any(char::is_control)
                        })
            })
    }

    /// The route for one of this channel's models at a thinking level; with
    /// no level the channel's reasoning choice applies.
    pub fn route_for(
        &self,
        model_id: &str,
        level: Option<&str>,
    ) -> Result<ModelRoute, ChannelError> {
        let model = self
            .effective_models()
            .into_iter()
            .find(|model| model.id == model_id)
            .ok_or(ChannelError::Invalid)?;
        let protocol_override = model.protocol.as_deref().unwrap_or(self.protocol.as_str());
        let route = self
            .route_with_model_protocol(&model.id, protocol_override)?
            .with_max_output_tokens(model.max_output_tokens);
        // A remembered refusal was seen for the channel's own model only.
        let route = route.with_reasoning_switch_unsupported(
            self.reasoning_switch_unsupported && model.id == self.model,
        );
        Ok(match level {
            None => route.with_reasoning(self.reasoning_mode),
            Some("off") => route.with_reasoning(ReasoningMode::Off),
            Some(level) => {
                let value = model
                    .reasoning_levels
                    .iter()
                    .find(|candidate| candidate.level == level)
                    .ok_or(ChannelError::Invalid)?
                    .value
                    .clone();
                route
                    .with_reasoning(ReasoningMode::ProviderDefault)
                    .with_reasoning_effort(value)
            }
        })
    }

    fn route_with_model_protocol(
        &self,
        model: &str,
        protocol_name: &str,
    ) -> Result<ModelRoute, ChannelError> {
        if self.id.is_empty()
            || self.id.len() > 64
            || is_reserved_channel_id(&self.id)
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !self.valid_models()
        {
            return Err(ChannelError::Invalid);
        }
        let protocol = parse_protocol(protocol_name).ok_or(ChannelError::Invalid)?;
        ModelRoute::new(protocol, &self.base_url, model).map_err(|_| ChannelError::Invalid)
    }

    /// Same protocol, base URL and model: what the refusal was seen for.
    fn same_endpoint(&self, other: &Self) -> bool {
        self.protocol == other.protocol
            && self.base_url == other.base_url
            && self.model == other.model
    }

    /// Validates channel metadata on save/load, permitting an empty default
    /// `model` for freshly authenticated channels before model selection.
    pub fn validate_channel_metadata(&self) -> Result<(), ChannelError> {
        if self.id.is_empty()
            || self.id.len() > 64
            || is_reserved_channel_id(&self.id)
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !self.valid_models()
        {
            return Err(ChannelError::Invalid);
        }
        let channel_protocol = parse_protocol(&self.protocol).ok_or(ChannelError::Invalid)?;
        crate::transport::Endpoint::new(&self.base_url, channel_protocol)
            .map_err(|_| ChannelError::Invalid)?;
        if !self.model.is_empty() {
            self.validated_route()?;
        } else {
            for m in &self.models {
                let p_name = m.protocol.as_deref().unwrap_or(self.protocol.as_str());
                self.route_with_model_protocol(&m.id, p_name)?;
            }
        }
        Ok(())
    }

    pub fn validated_route(&self) -> Result<ModelRoute, ChannelError> {
        let matched = self
            .effective_models()
            .into_iter()
            .find(|m| m.id == self.model);
        let model_cap = matched.as_ref().and_then(|m| m.max_output_tokens);
        let protocol_override = matched
            .as_ref()
            .and_then(|m| m.protocol.as_deref())
            .unwrap_or(self.protocol.as_str());
        self.route_with_model_protocol(&self.model, protocol_override)
            .map(|route| {
                route
                    .with_max_output_tokens(model_cap)
                    .with_reasoning(self.reasoning_mode)
                    .with_reasoning_switch_unsupported(self.reasoning_switch_unsupported)
            })
    }
}

/// The channel a run's model was opened from, for writing back a refused
/// reasoning switch (`agent::KeyedModel::remembering`).
pub struct ChannelMemory {
    store: ChannelStore,
    channel: ChannelConfig,
}
impl ChannelMemory {
    pub fn new(store: ChannelStore, channel: ChannelConfig) -> Self {
        Self { store, channel }
    }
    pub fn remember_reasoning_switch_unsupported(&self) -> Result<bool, ChannelError> {
        self.store
            .remember_reasoning_switch_unsupported(&self.channel)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultChannel {
    channel_id: String,
}

pub trait ChannelKeys {
    fn save_key(&self, channel_id: &str, key: &str) -> Result<(), ChannelError>;
    fn has_key(&self, channel_id: &str) -> Result<bool, ChannelError>;
    /// Removes the saved key of a channel; `Ok(false)` when none was saved.
    /// Implementations must not decode or return the saved value, and a value
    /// that cannot be decoded must not make the key undeletable.
    fn delete_key(&self, channel_id: &str) -> Result<bool, ChannelError>;
}

#[derive(Debug)]
pub enum ChannelError {
    Invalid,
    Unavailable,
}

pub struct ChannelStore {
    root: PathBuf,
}
impl ChannelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn save(
        &self,
        mut config: ChannelConfig,
        key: Option<&str>,
        keys: &impl ChannelKeys,
    ) -> Result<(), ChannelError> {
        config.validate_channel_metadata()?;
        let _write = write_lock();
        let mut values = self.load()?;
        // A remembered refusal belongs to the settings it was seen for: kept
        // when protocol, base URL and model are unchanged, never set here.
        config.reasoning_switch_unsupported = values.iter().any(|item| {
            item.id == config.id && item.reasoning_switch_unsupported && item.same_endpoint(&config)
        });
        if let Some(existing) = values.iter_mut().find(|item| item.id == config.id) {
            *existing = config.clone();
        } else if values.len() < 16 {
            values.push(config.clone());
        } else {
            return Err(ChannelError::Invalid);
        }
        if key.is_some_and(|key| key.is_empty() || key.len() > 16_384) {
            return Err(ChannelError::Invalid);
        }
        self.write_channels(&values)?;
        // A failed metadata write must not leave an orphaned key. The credential
        // service remains the sole owner; never serialize it into this file.
        if let Some(key) = key {
            keys.save_key(&config.id, key)?;
        }
        Ok(())
    }
    /// The channel list is replaced atomically; secrets never enter this file.
    fn write_channels(&self, values: &[ChannelConfig]) -> Result<(), ChannelError> {
        fs::create_dir_all(&self.root).map_err(|_| ChannelError::Unavailable)?;
        let data = serde_json::to_vec(values).map_err(|_| ChannelError::Invalid)?;
        let tmp = self.root.join("agent-channels.tmp");
        fs::write(&tmp, data).map_err(|_| ChannelError::Unavailable)?;
        fs::rename(&tmp, self.root.join("agent-channels.json"))
            .map_err(|_| ChannelError::Unavailable)
    }
    /// Removes one channel and everything that belongs to it: the saved key
    /// goes first so a crash never leaves a credential without its route, then
    /// the metadata entry, then the default pointer when it named this channel.
    /// A corrupt pointer is treated as none and removed: failing here, after
    /// the entry is already gone, would make every retry report an unknown
    /// channel.
    pub fn delete(&self, channel_id: &str, keys: &impl ChannelKeys) -> Result<(), ChannelError> {
        if is_reserved_channel_id(channel_id) {
            return Err(ChannelError::Invalid);
        }
        let _write = write_lock();
        let mut values = self.load()?;
        if !values.iter().any(|channel| channel.id == channel_id) {
            return Err(ChannelError::Invalid);
        }
        keys.delete_key(channel_id)?;
        values.retain(|channel| channel.id != channel_id);
        self.write_channels(&values)?;
        let remove_pointer = match self.default_pointer() {
            Ok(pointer) => pointer.as_deref() == Some(channel_id),
            Err(ChannelError::Invalid) => true,
            Err(error) => return Err(error),
        };
        if remove_pointer {
            fs::remove_file(self.root.join("agent-channel-default.json"))
                .map_err(|_| ChannelError::Unavailable)?;
        }
        Ok(())
    }
    fn load(&self) -> Result<Vec<ChannelConfig>, ChannelError> {
        let path = self.root.join("agent-channels.json");
        if !path.exists() {
            return Ok(vec![]);
        }
        let bytes = fs::read(path).map_err(|_| ChannelError::Unavailable)?;
        if bytes.len() > 32 * 1024 {
            return Err(ChannelError::Invalid);
        }
        let channels: Vec<ChannelConfig> =
            serde_json::from_slice(&bytes).map_err(|_| ChannelError::Invalid)?;
        if channels.len() > 16
            || channels
                .iter()
                .any(|channel| channel.validate_channel_metadata().is_err())
        {
            return Err(ChannelError::Invalid);
        }
        Ok(channels)
    }
    /// Records that the provider refused the reasoning switch for `used`
    /// (the settings a run opened its model from). Only a saved channel with
    /// the same id, protocol, base URL and model is marked; `Ok(false)` when
    /// the channel was deleted or changed meanwhile.
    pub fn remember_reasoning_switch_unsupported(
        &self,
        used: &ChannelConfig,
    ) -> Result<bool, ChannelError> {
        let _write = write_lock();
        let mut values = self.load()?;
        let Some(saved) = values
            .iter_mut()
            .find(|item| item.id == used.id && item.same_endpoint(used))
        else {
            return Ok(false);
        };
        if !saved.reasoning_switch_unsupported {
            saved.reasoning_switch_unsupported = true;
            self.write_channels(&values)?;
        }
        Ok(true)
    }
    /// One saved channel by id, without asking the credential service.
    pub fn saved(&self, channel_id: &str) -> Result<Option<ChannelConfig>, ChannelError> {
        Ok(self
            .load()?
            .into_iter()
            .find(|channel| channel.id == channel_id))
    }
    pub fn list(
        &self,
        keys: &impl ChannelKeys,
    ) -> Result<Vec<(ChannelConfig, bool)>, ChannelError> {
        self.load()?
            .into_iter()
            .map(|channel| {
                let ready = keys.has_key(&channel.id)?;
                Ok((channel, ready))
            })
            .collect()
    }
    /// Marks one saved channel as the one new runs use. The choice lives in
    /// its own small file so the channel list format stays unchanged.
    pub fn set_default(&self, channel_id: &str) -> Result<(), ChannelError> {
        if !self.load()?.iter().any(|channel| channel.id == channel_id) {
            return Err(ChannelError::Invalid);
        }
        let data = serde_json::to_vec(&DefaultChannel {
            channel_id: channel_id.to_owned(),
        })
        .map_err(|_| ChannelError::Invalid)?;
        let tmp = self.root.join("agent-channel-default.tmp");
        fs::write(&tmp, data).map_err(|_| ChannelError::Unavailable)?;
        fs::rename(&tmp, self.root.join("agent-channel-default.json"))
            .map_err(|_| ChannelError::Unavailable)
    }
    /// The raw default pointer, whether or not it still names a saved channel.
    fn default_pointer(&self) -> Result<Option<String>, ChannelError> {
        let path = self.root.join("agent-channel-default.json");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|_| ChannelError::Unavailable)?;
        if bytes.len() > 1024 {
            return Err(ChannelError::Invalid);
        }
        let saved: DefaultChannel =
            serde_json::from_slice(&bytes).map_err(|_| ChannelError::Invalid)?;
        Ok(Some(saved.channel_id))
    }
    /// The saved default channel id, if it still names a saved channel.
    pub fn default_id(&self) -> Result<Option<String>, ChannelError> {
        let Some(pointer) = self.default_pointer()? else {
            return Ok(None);
        };
        Ok(self
            .load()?
            .into_iter()
            .any(|channel| channel.id == pointer)
            .then_some(pointer))
    }
    /// The channel a new run uses: the default one (only if its key is
    /// saved; it never silently falls back), otherwise the first channel
    /// with a saved key.
    pub fn select_for_run(
        &self,
        keys: &impl ChannelKeys,
    ) -> Result<Option<ChannelConfig>, ChannelError> {
        let default = self.default_id()?;
        let channels = self.list(keys)?;
        Ok(match default {
            Some(id) => channels
                .into_iter()
                .find(|(channel, ready)| channel.id == id && *ready),
            None => channels.into_iter().find(|(_, ready)| *ready),
        }
        .map(|(channel, _)| channel))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}
