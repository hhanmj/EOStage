use insar_agent_model::{
    channels::{
        ChannelConfig, ChannelError, ChannelKeys, ChannelModel, ChannelStore, ReasoningLevel,
    },
    ReasoningMode,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Default)]
struct FakeKeyring {
    saved: Arc<Mutex<Vec<(String, String)>>>,
    deleted: Arc<Mutex<Vec<String>>>,
}
impl ChannelKeys for FakeKeyring {
    fn save_key(&self, id: &str, key: &str) -> Result<(), ChannelError> {
        self.saved
            .lock()
            .unwrap()
            .push((id.to_owned(), key.to_owned()));
        Ok(())
    }
    fn has_key(&self, id: &str) -> Result<bool, ChannelError> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == id)
            && !self.deleted.lock().unwrap().iter().any(|name| name == id))
    }
    fn delete_key(&self, id: &str) -> Result<bool, ChannelError> {
        self.deleted.lock().unwrap().push(id.to_owned());
        Ok(true)
    }
}
fn root() -> PathBuf {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agent-channels-{}-{}-{seq}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn channel_config_is_local_and_secret_only_enters_keyring_not_file_or_queries() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    let config = ChannelConfig {
        id: "route-a".into(),
        protocol: "chat_completions".into(),
        base_url: "https://example.invalid/v1".into(),
        model: "m".into(),
        reasoning_mode: ReasoningMode::Off,
        reasoning_switch_unsupported: false,
        models: vec![],
    };
    store
        .save(config.clone(), Some("not-a-real-secret"), &keys)
        .unwrap();
    assert_eq!(store.list(&keys).unwrap(), vec![(config, true)]);
    let contents = fs::read_to_string(folder.join("agent-channels.json")).unwrap();
    assert!(!contents.contains("not-a-real-secret"));
    assert!(!format!("{:?}", store.list(&keys).unwrap()).contains("not-a-real-secret"));
}
#[test]
fn unwritable_channel_metadata_never_mutates_the_keyring() {
    let root = root();
    fs::write(&root, b"fixture-file-not-directory").unwrap();
    let store = ChannelStore::new(&root);
    let keys = FakeKeyring::default();
    let config = ChannelConfig {
        id: "route-b".into(),
        protocol: "responses".into(),
        base_url: "https://example.invalid/v1".into(),
        model: "m".into(),
        reasoning_mode: ReasoningMode::Off,
        reasoning_switch_unsupported: false,
        models: vec![],
    };
    assert!(store.save(config, Some("fixture-only-key"), &keys).is_err());
    assert!(
        keys.saved.lock().unwrap().is_empty(),
        "key was saved without its route"
    );
}

#[test]
fn channel_url_with_query_or_embedded_user_is_rejected_before_credential_write() {
    let store = ChannelStore::new(root());
    let keys = FakeKeyring::default();
    for url in [
        "https://example.invalid/v1?token=fixture",
        "http://external.invalid/v1",
        "https://user:password@example.invalid/v1",
    ] {
        assert!(store
            .save(
                ChannelConfig {
                    id: "x".into(),
                    protocol: "responses".into(),
                    base_url: url.into(),
                    model: "m".into(),
                    reasoning_mode: ReasoningMode::Off,
                    reasoning_switch_unsupported: false,
                    models: vec![],
                },
                Some("fixture"),
                &keys
            )
            .is_err());
    }
    assert!(keys.saved.lock().unwrap().is_empty());
}

fn channel(id: &str) -> ChannelConfig {
    ChannelConfig {
        id: id.into(),
        protocol: "chat_completions".into(),
        base_url: "https://example.invalid/v1".into(),
        model: "m".into(),
        reasoning_mode: ReasoningMode::Off,
        reasoning_switch_unsupported: false,
        models: vec![],
    }
}

#[test]
fn runs_use_the_default_channel_or_else_the_first_one_with_a_key() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    store.save(channel("no-key"), None, &keys).unwrap();
    store
        .save(channel("first-ready"), Some("fixture-a"), &keys)
        .unwrap();
    store
        .save(channel("second-ready"), Some("fixture-b"), &keys)
        .unwrap();
    // No default: the first channel that has a key.
    assert_eq!(store.default_id().unwrap(), None);
    assert_eq!(
        store.select_for_run(&keys).unwrap().unwrap().id,
        "first-ready"
    );
    // A default is used even when it is not the first ready channel, and it
    // survives a new store instance (it is persisted).
    store.set_default("second-ready").unwrap();
    let reopened = ChannelStore::new(&folder);
    assert_eq!(
        reopened.default_id().unwrap().as_deref(),
        Some("second-ready")
    );
    assert_eq!(
        reopened.select_for_run(&keys).unwrap().unwrap().id,
        "second-ready"
    );
    // A default without a key does not silently fall back to another channel.
    store.set_default("no-key").unwrap();
    assert!(store.select_for_run(&keys).unwrap().is_none());
    // Only a saved channel can be the default.
    assert!(matches!(
        store.set_default("never-saved"),
        Err(ChannelError::Invalid)
    ));
    assert_eq!(store.default_id().unwrap().as_deref(), Some("no-key"));
    let file = fs::read_to_string(folder.join("agent-channel-default.json")).unwrap();
    assert!(!file.contains("fixture-"));
}

#[test]
fn channels_delete_removes_the_route_its_key_and_a_default_pointer_to_it() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    store
        .save(channel("route-a"), Some("fixture-a"), &keys)
        .unwrap();
    store
        .save(channel("route-b"), Some("fixture-b"), &keys)
        .unwrap();
    store.set_default("route-a").unwrap();

    store.delete("route-a", &keys).unwrap();

    assert_eq!(
        store
            .list(&keys)
            .unwrap()
            .into_iter()
            .map(|(channel, _)| channel.id)
            .collect::<Vec<_>>(),
        vec!["route-b".to_owned()]
    );
    // The pointer file is cleared, so new runs fall back to the first channel
    // with a saved key instead of the removed one.
    assert_eq!(store.default_id().unwrap(), None);
    assert!(!folder.join("agent-channel-default.json").exists());
    assert_eq!(store.select_for_run(&keys).unwrap().unwrap().id, "route-b");
    // The fake credential store received exactly one delete, for the deleted
    // channel's account; the remaining channel's key is untouched.
    assert_eq!(*keys.deleted.lock().unwrap(), vec!["route-a".to_owned()]);
    assert!(keys.has_key("route-b").unwrap());
    // Deleting an unknown channel changes nothing.
    assert!(matches!(
        store.delete("never-saved", &keys),
        Err(ChannelError::Invalid)
    ));
    assert_eq!(keys.deleted.lock().unwrap().len(), 1);
    assert!(folder.join("agent-channels.json").exists());
}

#[test]
fn channels_delete_keeps_the_default_pointer_when_it_names_another_channel() {
    let store = ChannelStore::new(root());
    let keys = FakeKeyring::default();
    store
        .save(channel("route-a"), Some("fixture-a"), &keys)
        .unwrap();
    store
        .save(channel("route-b"), Some("fixture-b"), &keys)
        .unwrap();
    store.set_default("route-b").unwrap();

    store.delete("route-a", &keys).unwrap();

    assert_eq!(store.default_id().unwrap().as_deref(), Some("route-b"));
    assert_eq!(*keys.deleted.lock().unwrap(), vec!["route-a".to_owned()]);
}

/// A credential store whose delete always fails, as when Windows Credential
/// Manager is locked or unavailable.
struct FailingDelete(FakeKeyring);
impl ChannelKeys for FailingDelete {
    fn save_key(&self, id: &str, key: &str) -> Result<(), ChannelError> {
        self.0.save_key(id, key)
    }
    fn has_key(&self, id: &str) -> Result<bool, ChannelError> {
        self.0.has_key(id)
    }
    fn delete_key(&self, _id: &str) -> Result<bool, ChannelError> {
        Err(ChannelError::Unavailable)
    }
}

#[test]
fn channels_delete_leaves_metadata_and_default_untouched_when_the_key_delete_fails() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FailingDelete(FakeKeyring::default());
    store
        .save(channel("route-a"), Some("fixture-a"), &keys)
        .unwrap();
    store.set_default("route-a").unwrap();
    let channels_before = fs::read(folder.join("agent-channels.json")).unwrap();
    let default_before = fs::read(folder.join("agent-channel-default.json")).unwrap();

    assert!(matches!(
        store.delete("route-a", &keys),
        Err(ChannelError::Unavailable)
    ));

    // The key goes first: when it cannot be removed, the route and the default
    // pointer stay byte-for-byte, so a retry still finds the channel.
    assert_eq!(
        fs::read(folder.join("agent-channels.json")).unwrap(),
        channels_before
    );
    assert_eq!(
        fs::read(folder.join("agent-channel-default.json")).unwrap(),
        default_before
    );
    assert_eq!(store.default_id().unwrap().as_deref(), Some("route-a"));
    assert!(store.delete("route-a", &keys.0).is_ok());
}

#[test]
fn channels_delete_treats_a_corrupt_default_pointer_as_none_and_removes_it() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    store
        .save(channel("route-a"), Some("fixture-a"), &keys)
        .unwrap();
    store
        .save(channel("route-b"), Some("fixture-b"), &keys)
        .unwrap();
    fs::write(folder.join("agent-channel-default.json"), b"{not json").unwrap();

    store.delete("route-a", &keys).unwrap();

    assert!(!folder.join("agent-channel-default.json").exists());
    assert_eq!(store.default_id().unwrap(), None);
    assert_eq!(
        store
            .list(&keys)
            .unwrap()
            .into_iter()
            .map(|(channel, _)| channel.id)
            .collect::<Vec<_>>(),
        vec!["route-b".to_owned()]
    );
    assert_eq!(*keys.deleted.lock().unwrap(), vec!["route-a".to_owned()]);
}

#[test]
fn a_channel_saved_before_the_reasoning_option_reads_as_off() {
    let folder = root();
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("agent-channels.json"),
        br#"[{"id":"legacy","protocol":"chat_completions","base_url":"https://example.invalid/v1","model":"glm-5.3-flash"}]"#,
    )
    .unwrap();
    let store = ChannelStore::new(&folder);
    let legacy = store
        .saved("legacy")
        .unwrap()
        .expect("legacy channel loads");
    assert_eq!(legacy.reasoning_mode, ReasoningMode::Off);
    let route = legacy.validated_route().unwrap();
    assert_eq!(route.reasoning, ReasoningMode::Off);
    let (_, body) = route.request(&[], &[]).unwrap();
    assert_eq!(body["thinking"], serde_json::json!({"type": "disabled"}));
}

#[test]
fn the_reasoning_mode_is_saved_with_the_channel_and_reaches_its_route() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    let mut config = channel("thinking");
    config.model = "glm-5.3-flash".into();
    config.reasoning_mode = ReasoningMode::ProviderDefault;
    store.save(config, None, &keys).unwrap();
    let contents = fs::read_to_string(folder.join("agent-channels.json")).unwrap();
    assert!(
        contents.contains(r#""reasoning_mode":"default""#),
        "{contents}"
    );
    let saved = store.saved("thinking").unwrap().unwrap();
    assert_eq!(saved.reasoning_mode, ReasoningMode::ProviderDefault);
    let (_, body) = saved.validated_route().unwrap().request(&[], &[]).unwrap();
    assert!(body.get("thinking").is_none(), "{body}");
    assert_eq!(store.saved("absent").unwrap(), None);
    // An unknown stored value is refused like any other malformed channel.
    fs::write(
        folder.join("agent-channels.json"),
        br#"[{"id":"x","protocol":"chat_completions","base_url":"https://example.invalid/v1","model":"m","reasoning_mode":"high"}]"#,
    )
    .unwrap();
    assert!(matches!(store.saved("x"), Err(ChannelError::Invalid)));
}

#[test]
fn an_off_channel_is_written_without_the_field_so_older_builds_still_load_it() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    store.save(channel("plain"), None, &keys).unwrap();
    let contents = fs::read_to_string(folder.join("agent-channels.json")).unwrap();
    assert!(!contents.contains("reasoning_mode"), "{contents}");
    // The pre-3.31.0 shape (deny_unknown_fields, no reasoning_mode) reads it.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Before {
        id: String,
        protocol: String,
        base_url: String,
        model: String,
    }
    let before: Vec<Before> = serde_json::from_str(&contents).unwrap();
    assert_eq!(before.len(), 1);
}

/// W3o item 1 (L4 r9 P2-1): a channel whose provider refused the reasoning
/// switch remembers it; its route then sends no switch and a larger output
/// budget. A change of protocol, base URL or model forgets it; a remembered
/// refusal for settings that changed meanwhile is not written.
#[test]
fn a_refused_reasoning_switch_is_remembered_per_channel_until_its_endpoint_changes() {
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    let mut config = channel("glm");
    config.model = "glm-5.3-flash".into();
    store.save(config.clone(), None, &keys).unwrap();
    let contents = fs::read_to_string(folder.join("agent-channels.json")).unwrap();
    assert!(
        !contents.contains("reasoning_switch_unsupported"),
        "not written while false, so older builds still load the file: {contents}"
    );
    assert!(store
        .remember_reasoning_switch_unsupported(&config)
        .unwrap());
    let saved = store.saved("glm").unwrap().unwrap();
    assert!(saved.reasoning_switch_unsupported);
    let route = saved.validated_route().unwrap();
    assert!(!route.sends_reasoning_switch());
    assert!(route.reasoning_switch_refused());
    let (_, body) = route.request(&[], &[]).unwrap();
    assert!(body.get("thinking").is_none(), "{body}");
    assert_eq!(body["max_tokens"], 8192, "{body}");

    // Saving the same endpoint (the form never sends the flag) keeps it;
    // the reasoning choice is not part of the endpoint.
    let mut same = config.clone();
    same.reasoning_mode = ReasoningMode::ProviderDefault;
    store.save(same, None, &keys).unwrap();
    assert!(
        store
            .saved("glm")
            .unwrap()
            .unwrap()
            .reasoning_switch_unsupported
    );
    store.save(config.clone(), None, &keys).unwrap();
    assert!(
        store
            .saved("glm")
            .unwrap()
            .unwrap()
            .reasoning_switch_unsupported
    );
    // A caller cannot set it by saving.
    let mut other = channel("other");
    other.reasoning_switch_unsupported = true;
    store.save(other, None, &keys).unwrap();
    assert!(
        !store
            .saved("other")
            .unwrap()
            .unwrap()
            .reasoning_switch_unsupported
    );

    for change in ["base_url", "model", "protocol"] {
        store
            .remember_reasoning_switch_unsupported(&config)
            .unwrap();
        assert!(
            store
                .saved("glm")
                .unwrap()
                .unwrap()
                .reasoning_switch_unsupported
        );
        let mut changed = config.clone();
        match change {
            "base_url" => changed.base_url = "https://other.invalid/v1".into(),
            "model" => changed.model = "glm-4.6".into(),
            _ => changed.protocol = "responses".into(),
        }
        store.save(changed, None, &keys).unwrap();
        let saved = store.saved("glm").unwrap().unwrap();
        assert!(!saved.reasoning_switch_unsupported, "{change}");
        // A run that used the old settings does not mark the new ones.
        assert!(!store
            .remember_reasoning_switch_unsupported(&config)
            .unwrap());
        assert!(
            !store
                .saved("glm")
                .unwrap()
                .unwrap()
                .reasoning_switch_unsupported
        );
        store.save(config.clone(), None, &keys).unwrap();
    }
    // An unknown channel (deleted meanwhile) is not recreated.
    assert!(!store
        .remember_reasoning_switch_unsupported(&channel("gone"))
        .unwrap());
    assert_eq!(store.saved("gone").unwrap(), None);
}

/// The flag as stored by this build loads; `false` reads the same as absent.
#[test]
fn a_remembered_refusal_loads_from_the_channel_file() {
    let folder = root();
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("agent-channels.json"),
        br#"[{"id":"r","protocol":"chat_completions","base_url":"https://example.invalid/v1","model":"glm-5.3-flash","reasoning_switch_unsupported":true},{"id":"f","protocol":"chat_completions","base_url":"https://example.invalid/v1","model":"glm-5.3-flash","reasoning_switch_unsupported":false}]"#,
    )
    .unwrap();
    let store = ChannelStore::new(&folder);
    assert!(
        store
            .saved("r")
            .unwrap()
            .unwrap()
            .reasoning_switch_unsupported
    );
    assert!(
        !store
            .saved("f")
            .unwrap()
            .unwrap()
            .reasoning_switch_unsupported
    );
}

/// A model with no channel to write to still stops sending the switch for
/// the rest of its run.
#[test]
fn a_keyed_model_without_a_channel_remembers_the_refusal_for_its_run() {
    use insar_agent_model::{
        agent::{KeyedModel, Model},
        transport::{ModelClient, ModelRoute},
        Protocol, Zeroizing,
    };
    let route = ModelRoute::new(
        Protocol::ChatCompletions,
        "https://example.invalid/v1",
        "glm-5.3-flash",
    )
    .unwrap();
    let model = KeyedModel::new(
        ModelClient::new(std::time::Duration::from_secs(1)).unwrap(),
        route,
        Zeroizing::new("fixture-only".into()),
    );
    let before = model.request_body(&[], &[]).unwrap().unwrap();
    assert_eq!(before["thinking"], serde_json::json!({"type": "disabled"}));
    assert!(model.without_reasoning_switch().is_some());
    model.remember_reasoning_switch_unsupported();
    assert!(model.reasoning_switch_unsupported());
    assert!(model.without_reasoning_switch().is_none());
    let after = model.request_body(&[], &[]).unwrap().unwrap();
    assert!(after.get("thinking").is_none(), "{after}");
    assert_eq!(after["max_tokens"], 8192, "{after}");
}

/// The run's model writes the refusal back through the channel it was
/// opened from, and reports a remembered refusal to the host.
#[test]
fn the_keyed_model_remembers_a_refusal_for_its_channel() {
    use insar_agent_model::{
        agent::{KeyedModel, Model},
        channels::ChannelMemory,
        transport::ModelClient,
        Zeroizing,
    };
    let folder = root();
    let store = ChannelStore::new(&folder);
    let keys = FakeKeyring::default();
    let mut config = channel("glm");
    config.model = "glm-5.3-flash".into();
    store.save(config.clone(), None, &keys).unwrap();
    let client = ModelClient::new(std::time::Duration::from_secs(1)).unwrap();
    let model = KeyedModel::new(
        client.clone(),
        config.validated_route().unwrap(),
        Zeroizing::new("fixture-only".into()),
    )
    .remembering(ChannelMemory::new(
        ChannelStore::new(&folder),
        config.clone(),
    ));
    assert!(!model.reasoning_switch_unsupported());
    let resend = model
        .without_reasoning_switch()
        .expect("glm sends a switch");
    let body = resend.request_body(&[], &[]).unwrap().unwrap();
    assert!(body.get("thinking").is_none(), "{body}");
    assert_eq!(body["max_tokens"], 8192, "the resend lets the model think");
    model.remember_reasoning_switch_unsupported();
    // W3o patch: the same model, for the rest of its run, sends no switch
    // (L4 r9 conv1: 5 calls in one run, each refused and resent).
    assert!(model.reasoning_switch_unsupported());
    assert!(
        model.without_reasoning_switch().is_none(),
        "no second resend in the same run"
    );
    let body = model.request_body(&[], &[]).unwrap().unwrap();
    assert!(body.get("thinking").is_none(), "{body}");
    assert_eq!(body["max_tokens"], 8192, "{body}");
    let saved = store.saved("glm").unwrap().unwrap();
    assert!(saved.reasoning_switch_unsupported);
    let next = KeyedModel::new(
        client,
        saved.validated_route().unwrap(),
        Zeroizing::new("fixture-only".into()),
    );
    assert!(next.reasoning_switch_unsupported());
    assert!(
        next.without_reasoning_switch().is_none(),
        "no switch, no resend"
    );
}

fn listed(id: &str, protocol: &str) -> ChannelConfig {
    ChannelConfig {
        protocol: protocol.into(),
        models: vec![
            ChannelModel {
                id: "fixture-fast".into(),
                ..ChannelModel::default()
            },
            ChannelModel {
                id: "fixture-deep".into(),
                name: Some("深度".into()),
                reasoning_levels: vec![
                    ReasoningLevel {
                        level: "off".into(),
                        value: None,
                    },
                    ReasoningLevel {
                        level: "high".into(),
                        value: Some(if protocol == "anthropic_messages" {
                            "8000".into()
                        } else {
                            "high".into()
                        }),
                    },
                    ReasoningLevel {
                        level: "provider".into(),
                        value: None,
                    },
                ],
                ..ChannelModel::default()
            },
        ],
        ..channel(id)
    }
}

fn body_for(config: &ChannelConfig, model: &str, level: Option<&str>) -> serde_json::Value {
    let route = config.route_for(model, level).unwrap();
    assert_eq!(route.model, model);
    route.request(&[], &[]).unwrap().1
}

#[test]
fn a_channel_without_a_model_list_offers_its_one_model() {
    let plain = channel("plain");
    let models = plain.effective_models();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, plain.model);
    assert!(plain.route_for(&plain.model, None).is_ok());
    assert!(matches!(
        plain.route_for("not-listed", None),
        Err(ChannelError::Invalid)
    ));
}

#[test]
fn a_thinking_level_value_reaches_the_request_of_each_protocol() {
    let chat = listed("chat", "chat_completions");
    assert_eq!(
        body_for(&chat, "fixture-deep", Some("high"))["reasoning_effort"],
        "high"
    );
    assert!(body_for(&chat, "fixture-deep", Some("provider"))
        .get("reasoning_effort")
        .is_none());
    assert!(body_for(&chat, "fixture-fast", None)
        .get("reasoning_effort")
        .is_none());
    let responses = listed("responses", "responses");
    assert_eq!(
        body_for(&responses, "fixture-deep", Some("high"))["reasoning"]["effort"],
        "high"
    );
    let anthropic = listed("anthropic", "anthropic_messages");
    let body = body_for(&anthropic, "fixture-deep", Some("high"));
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 8000);
    assert_eq!(body["max_tokens"], 12096);
    assert!(matches!(
        chat.route_for("fixture-deep", Some("unknown")),
        Err(ChannelError::Invalid)
    ));
}

#[test]
fn a_model_list_is_saved_with_the_channel() {
    let store = ChannelStore::new(root());
    let keys = FakeKeyring::default();
    let mut rich = listed("saved", "chat_completions");
    rich.models[0].protocol = Some("google_gemini".into());
    rich.models[0].input_types = vec!["text".into(), "image".into(), "video".into(), "pdf".into()];
    rich.models[0].supports_structured_output = Some(true);
    rich.models[0].supports_web_search = Some(false);
    rich.models[0].supports_mid_turn_system = Some(true);
    rich.models[0].reasoning_param_mapping = Some("reasoning_effort".into());
    rich.models[0].is_free = Some(true);
    store.save(rich, None, &keys).unwrap();
    let saved = store.saved("saved").unwrap().unwrap();
    assert_eq!(saved.models.len(), 2);
    assert_eq!(saved.models[0].protocol.as_deref(), Some("google_gemini"));
    assert_eq!(
        saved.models[0].input_types,
        vec!["text", "image", "video", "pdf"]
    );
    assert_eq!(saved.models[0].supports_structured_output, Some(true));
    assert_eq!(saved.models[0].supports_web_search, Some(false));
    assert_eq!(saved.models[0].supports_mid_turn_system, Some(true));
    assert_eq!(
        saved.models[0].reasoning_param_mapping.as_deref(),
        Some("reasoning_effort")
    );
    assert_eq!(saved.models[0].is_free, Some(true));
    assert_eq!(saved.models[1].is_free, None);
    assert_eq!(
        saved
            .route_for(&saved.models[0].id, None)
            .unwrap()
            .request(&[], &[])
            .unwrap()
            .0
            .as_str(),
        "https://example.invalid/v1/models/fixture-fast:streamGenerateContent?alt=sse"
    );
    assert_eq!(saved.models[1].reasoning_levels.len(), 3);

    // Unsupported input_types like "audio" remain rejected.
    let mut bad = listed("bad", "chat_completions");
    bad.models[0].input_types = vec!["audio".into()];
    assert!(matches!(
        store.save(bad, None, &keys),
        Err(ChannelError::Invalid)
    ));

    // Fresh OAuth channel with empty model/list saves and loads cleanly.
    let mut empty_oauth = channel("oauth-empty");
    empty_oauth.protocol = "google_gemini".into();
    empty_oauth.base_url = "https://generativelanguage.googleapis.com/v1beta".into();
    empty_oauth.model.clear();
    empty_oauth.models.clear();
    store.save(empty_oauth, None, &keys).unwrap();
    let loaded_empty = store.saved("oauth-empty").unwrap().unwrap();
    assert!(loaded_empty.model.is_empty());
    assert!(loaded_empty.effective_models().is_empty());
}

#[test]
fn reserved_gee_channel_ids_are_rejected_without_touching_keyring() {
    let store = ChannelStore::new(root());
    let keys = FakeKeyring::default();
    keys.save_key("gee-credentials", "{\"type\":\"service_account\"}")
        .unwrap();
    keys.save_key("gee-proj-demo", "proj-demo").unwrap();

    for reserved in [
        "gee",
        "gee-credentials",
        "GEE-CREDENTIALS",
        "gee-proj-demo",
        "gee_oauth_token",
    ] {
        assert!(insar_agent_model::channels::is_reserved_channel_id(
            reserved
        ));
        assert!(matches!(
            store.save(
                listed(reserved, "chat_completions"),
                Some("sk-attempt"),
                &keys
            ),
            Err(ChannelError::Invalid)
        ));
        assert!(matches!(
            store.delete(reserved, &keys),
            Err(ChannelError::Invalid)
        ));
    }

    assert!(keys.has_key("gee-credentials").unwrap());
    assert!(keys.has_key("gee-proj-demo").unwrap());
    assert!(!insar_agent_model::channels::is_reserved_channel_id(
        "gemini-pro"
    ));
}
