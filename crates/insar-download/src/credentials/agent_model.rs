//! Model channel API keys in the system credential store, one generic
//! credential per channel: target `eostage:agent-model:<channel id>`, user
//! name = channel id. Windows keys generic credentials by target alone, so the
//! target itself must name the channel.
//!
//! Builds before this layout kept every channel under the one target
//! `eostage:agent-model` (the Python keyring layout: the channel saved last
//! on the bare target, earlier ones moved to `<id>@eostage:agent-model`).
//! A channel's legacy entry moves to the channel's own target the first time
//! that channel's key is looked up while the channel target is empty; legacy
//! entries are otherwise removed only when the channel's key is saved again or
//! deleted. Only channels in the channel list are ever looked up, so a legacy
//! entry whose user name names no channel is left alone. After the move,
//! builds that only know the shared target no longer see the channel's key.
use std::sync::{Mutex, MutexGuard, PoisonError};

use zeroize::Zeroizing;

use super::{platform_keyring, DesktopCredentialError, AGENT_MODEL_KEYRING_SERVICE};

/// Exact-target operations on the credential store; tests substitute an
/// in-memory store keyed by target, as Windows is.
pub(super) trait KeyTargets {
    /// The secret on `target` when its stored user name is `username`.
    fn read(
        &self,
        target: &str,
        username: &str,
    ) -> Result<Option<Zeroizing<String>>, DesktopCredentialError>;
    fn write(
        &self,
        target: &str,
        username: &str,
        secret: &str,
    ) -> Result<(), DesktopCredentialError>;
    /// Deletes `target` when its stored user name is `username`, without
    /// decoding the secret.
    fn delete(&self, target: &str, username: &str) -> Result<bool, DesktopCredentialError>;
    /// One diagnostic line; callers pass only event codes and channel ids.
    fn record(&self, event: &str);
}

struct SystemTargets;

impl KeyTargets for SystemTargets {
    fn read(
        &self,
        target: &str,
        username: &str,
    ) -> Result<Option<Zeroizing<String>>, DesktopCredentialError> {
        Ok(platform_keyring::read_owned(target, username)?
            .map(Zeroizing::new)
            .filter(|value| !value.trim().is_empty()))
    }
    fn write(
        &self,
        target: &str,
        username: &str,
        secret: &str,
    ) -> Result<(), DesktopCredentialError> {
        platform_keyring::write_owned(target, username, secret)
    }
    fn delete(&self, target: &str, username: &str) -> Result<bool, DesktopCredentialError> {
        platform_keyring::delete_owned(target, username)
    }
    fn record(&self, event: &str) {
        eprintln!("{event}");
    }
}

/// Serialises key changes so a migration cannot write an old key over one
/// saved at the same moment. Never held while a borrowed key is in use or
/// while an event is recorded. It guards no data, so a poisoned lock (a
/// panic elsewhere while it was held) is simply taken over.
static AGENT_MODEL_KEY_LOCK: Mutex<()> = Mutex::new(());

fn key_guard() -> MutexGuard<'static, ()> {
    AGENT_MODEL_KEY_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn account(channel_id: &str) -> Result<&str, DesktopCredentialError> {
    if channel_id.is_empty()
        || channel_id.len() > 64
        || !channel_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(DesktopCredentialError::InvalidInput);
    }
    Ok(channel_id)
}

/// The credential target that holds one channel's key.
pub(crate) fn agent_model_key_target(channel_id: &str) -> Result<String, DesktopCredentialError> {
    Ok(format!(
        "{AGENT_MODEL_KEYRING_SERVICE}:{}",
        account(channel_id)?
    ))
}

/// Targets earlier builds may have left a channel's key on, newest layout
/// first: the bare service, the Python collision target, and Rust keyring's
/// default Windows target.
fn legacy_targets(channel_id: &str) -> [String; 3] {
    [
        AGENT_MODEL_KEYRING_SERVICE.to_owned(),
        format!("{channel_id}@{AGENT_MODEL_KEYRING_SERVICE}"),
        format!("{channel_id}.{AGENT_MODEL_KEYRING_SERVICE}"),
    ]
}

fn remove_legacy(
    targets: &impl KeyTargets,
    channel_id: &str,
) -> Result<bool, DesktopCredentialError> {
    let mut removed = false;
    for legacy in legacy_targets(channel_id) {
        removed |= targets.delete(&legacy, channel_id)?;
    }
    Ok(removed)
}

/// The channel's key, after moving a legacy entry to the channel's target,
/// and the event to record once the lock is released. Legacy entries are
/// removed only when this call moved one: while the channel target exists,
/// a legacy entry may hold a key re-entered in an older build, which must
/// not be deleted behind the user's back.
fn migrated_key(
    targets: &impl KeyTargets,
    channel_id: &str,
) -> Result<(Option<Zeroizing<String>>, Option<String>), DesktopCredentialError> {
    let target = agent_model_key_target(channel_id)?;
    if let Some(key) = targets.read(&target, channel_id)? {
        return Ok((Some(key), None));
    }
    for legacy in legacy_targets(channel_id) {
        if let Some(secret) = targets.read(&legacy, channel_id)? {
            // Write first: a crash before the delete leaves both copies, and
            // the channel target then wins.
            targets.write(&target, channel_id, &secret)?;
            remove_legacy(targets, channel_id)?;
            return Ok((
                Some(secret),
                Some(format!("AGENT_MODEL_KEY_MIGRATED channel={channel_id}")),
            ));
        }
    }
    Ok((None, None))
}

fn store_with(
    targets: &impl KeyTargets,
    channel_id: &str,
    key: &str,
) -> Result<(), DesktopCredentialError> {
    let target = agent_model_key_target(channel_id)?;
    if key.trim().is_empty() || key.len() > 16_384 {
        return Err(DesktopCredentialError::InvalidInput);
    }
    let removed = {
        let _guard = key_guard();
        targets.write(&target, channel_id, key)?;
        // A stale legacy key must not come back after this key is deleted.
        remove_legacy(targets, channel_id)?
    };
    if removed {
        targets.record(&format!(
            "AGENT_MODEL_KEY_LEGACY_REMOVED channel={channel_id}"
        ));
    }
    Ok(())
}

/// Looks the key up under the lock and records any migration after it.
fn locked_key(
    targets: &impl KeyTargets,
    channel_id: &str,
) -> Result<Option<Zeroizing<String>>, DesktopCredentialError> {
    let (key, event) = {
        let _guard = key_guard();
        migrated_key(targets, channel_id)?
    };
    if let Some(event) = event {
        targets.record(&event);
    }
    Ok(key)
}

fn ready_with(targets: &impl KeyTargets, channel_id: &str) -> Result<bool, DesktopCredentialError> {
    Ok(locked_key(targets, channel_id)?.is_some())
}

fn with_key_with<T>(
    targets: &impl KeyTargets,
    channel_id: &str,
    action: impl FnOnce(&str) -> T,
) -> Result<Option<T>, DesktopCredentialError> {
    Ok(locked_key(targets, channel_id)?.map(|key| action(key.as_str())))
}

fn delete_with(
    targets: &impl KeyTargets,
    channel_id: &str,
) -> Result<bool, DesktopCredentialError> {
    let target = agent_model_key_target(channel_id)?;
    let _guard = key_guard();
    let removed = targets.delete(&target, channel_id)?;
    Ok(remove_legacy(targets, channel_id)? || removed)
}

/// Save a model channel API key in the system credential store only.
pub fn store_desktop_agent_model_key(
    channel_id: &str,
    key: &str,
) -> Result<(), DesktopCredentialError> {
    store_with(&SystemTargets, channel_id, key)
}

/// Whether a key is saved; the value itself is never projected.
pub fn desktop_agent_model_key_ready(channel_id: &str) -> Result<bool, DesktopCredentialError> {
    ready_with(&SystemTargets, channel_id)
}

/// Borrow a model key inside a native closure for one model request or run.
pub fn with_desktop_agent_model_key<T>(
    channel_id: &str,
    action: impl FnOnce(&str) -> T,
) -> Result<Option<T>, DesktopCredentialError> {
    with_key_with(&SystemTargets, channel_id, action)
}

/// Delete a model channel API key, and any legacy entry whose user name is
/// this channel, from the system credential store; `Ok(false)` means no key
/// was saved. Entries are matched by target and user name; on Windows the OS
/// copies the saved value into its read buffer, which is zeroed and freed
/// without being decoded or logged, so an undecodable value is still deleted.
pub fn delete_desktop_agent_model_key(channel_id: &str) -> Result<bool, DesktopCredentialError> {
    delete_with(&SystemTargets, channel_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    const LEGACY: &str = "eostage:agent-model";

    /// A generic-credential store keyed by target alone, like Windows.
    #[derive(Default)]
    struct FakeTargets {
        entries: RefCell<BTreeMap<String, (String, String)>>,
        events: RefCell<Vec<String>>,
        decodes: Cell<usize>,
        decoded: RefCell<Vec<String>>,
    }

    impl FakeTargets {
        fn seed(&self, target: &str, username: &str, secret: &str) {
            self.entries
                .borrow_mut()
                .insert(target.to_owned(), (username.to_owned(), secret.to_owned()));
        }
        fn targets(&self) -> Vec<String> {
            self.entries.borrow().keys().cloned().collect()
        }
        fn entry(&self, target: &str) -> Option<(String, String)> {
            self.entries.borrow().get(target).cloned()
        }
        fn events(&self) -> Vec<String> {
            self.events.borrow().clone()
        }
        fn key(&self, channel_id: &str) -> Option<String> {
            with_key_with(self, channel_id, str::to_owned).unwrap()
        }
    }

    impl KeyTargets for FakeTargets {
        fn read(
            &self,
            target: &str,
            username: &str,
        ) -> Result<Option<Zeroizing<String>>, DesktopCredentialError> {
            Ok(match self.entries.borrow().get(target) {
                Some((owner, secret)) if owner == username => {
                    self.decodes.set(self.decodes.get() + 1);
                    self.decoded.borrow_mut().push(target.to_owned());
                    Some(Zeroizing::new(secret.clone()))
                }
                _ => None,
            })
        }
        fn write(
            &self,
            target: &str,
            username: &str,
            secret: &str,
        ) -> Result<(), DesktopCredentialError> {
            self.seed(target, username, secret);
            Ok(())
        }
        fn delete(&self, target: &str, username: &str) -> Result<bool, DesktopCredentialError> {
            let mut entries = self.entries.borrow_mut();
            if entries
                .get(target)
                .is_some_and(|(owner, _)| owner == username)
            {
                entries.remove(target);
                return Ok(true);
            }
            Ok(false)
        }
        fn record(&self, event: &str) {
            self.events.borrow_mut().push(event.to_owned());
        }
    }

    #[test]
    fn agent_model_key_target_names_the_channel() {
        assert_eq!(
            agent_model_key_target("l4-glm_2").unwrap(),
            "eostage:agent-model:l4-glm_2"
        );
        for invalid in ["", "a:b", "a b", &"x".repeat(65)] {
            assert!(matches!(
                agent_model_key_target(invalid),
                Err(DesktopCredentialError::InvalidInput)
            ));
        }
    }

    #[test]
    fn agent_model_keys_of_two_channels_live_on_distinct_targets() {
        let store = FakeTargets::default();
        store_with(&store, "route-a", "fixture-key-a").unwrap();
        store_with(&store, "route-b", "fixture-key-b").unwrap();

        assert_eq!(
            store.targets(),
            ["eostage:agent-model:route-a", "eostage:agent-model:route-b"]
        );
        assert_eq!(
            store.entry("eostage:agent-model:route-a"),
            Some(("route-a".to_owned(), "fixture-key-a".to_owned()))
        );
        assert!(ready_with(&store, "route-a").unwrap());
        assert!(ready_with(&store, "route-b").unwrap());
        assert_eq!(store.key("route-a").as_deref(), Some("fixture-key-a"));
        assert_eq!(store.key("route-b").as_deref(), Some("fixture-key-b"));
    }

    #[test]
    fn agent_model_key_delete_of_one_channel_keeps_the_other() {
        let store = FakeTargets::default();
        store_with(&store, "route-a", "fixture-key-a").unwrap();
        store_with(&store, "route-b", "fixture-key-b").unwrap();

        assert!(delete_with(&store, "route-a").unwrap());
        assert!(!ready_with(&store, "route-a").unwrap());
        assert_eq!(store.key("route-b").as_deref(), Some("fixture-key-b"));
        assert_eq!(store.targets(), ["eostage:agent-model:route-b"]);
        assert!(!delete_with(&store, "route-a").unwrap());
    }

    #[test]
    fn agent_model_legacy_keys_move_to_their_channel_targets_once() {
        let store = FakeTargets::default();
        // The layout earlier builds left after saving route-b, then route-a,
        // plus a leftover whose channel no longer exists.
        store.seed(LEGACY, "route-a", "fixture-key-a");
        store.seed("route-b@eostage:agent-model", "route-b", "fixture-key-b");
        store.seed("ghost@eostage:agent-model", "ghost", "fixture-key-g");

        assert!(ready_with(&store, "route-a").unwrap());
        assert_eq!(store.key("route-b").as_deref(), Some("fixture-key-b"));
        assert_eq!(
            store.targets(),
            [
                "eostage:agent-model:route-a",
                "eostage:agent-model:route-b",
                "ghost@eostage:agent-model",
            ]
        );
        assert_eq!(store.key("route-a").as_deref(), Some("fixture-key-a"));
        assert_eq!(
            store.events(),
            [
                "AGENT_MODEL_KEY_MIGRATED channel=route-a",
                "AGENT_MODEL_KEY_MIGRATED channel=route-b",
            ]
        );

        // A second pass changes nothing and records nothing.
        assert!(ready_with(&store, "route-a").unwrap());
        assert!(ready_with(&store, "route-b").unwrap());
        assert_eq!(store.targets().len(), 3);
        assert_eq!(store.events().len(), 2);
    }

    #[test]
    fn agent_model_channel_key_wins_and_a_lookup_leaves_legacy_entries_alone() {
        // An older build only sees the shared target: a key re-entered there
        // must not be deleted by a lookup in this build.
        let store = FakeTargets::default();
        store.seed("eostage:agent-model:route-a", "route-a", "fixture-key-new");
        store.seed(LEGACY, "route-a", "fixture-key-old");

        assert_eq!(store.key("route-a").as_deref(), Some("fixture-key-new"));
        assert!(ready_with(&store, "route-a").unwrap());
        assert_eq!(store.targets(), [LEGACY, "eostage:agent-model:route-a"]);
        assert_eq!(
            store.entry(LEGACY),
            Some(("route-a".to_owned(), "fixture-key-old".to_owned()))
        );
        assert!(store.events().is_empty());
    }

    #[test]
    fn agent_model_bare_legacy_entry_wins_over_the_collision_target() {
        let store = FakeTargets::default();
        store.seed(LEGACY, "route-a", "fixture-key-bare");
        store.seed(
            "route-a@eostage:agent-model",
            "route-a",
            "fixture-key-collision",
        );

        assert_eq!(store.key("route-a").as_deref(), Some("fixture-key-bare"));
        assert_eq!(store.targets(), ["eostage:agent-model:route-a"]);
        assert_eq!(
            store.entry("eostage:agent-model:route-a"),
            Some(("route-a".to_owned(), "fixture-key-bare".to_owned()))
        );
        assert_eq!(store.events(), ["AGENT_MODEL_KEY_MIGRATED channel=route-a"]);
    }

    #[test]
    fn agent_model_listed_channel_migrates_past_a_bare_target_of_an_unlisted_channel() {
        let store = FakeTargets::default();
        store.seed(LEGACY, "ghost", "fixture-key-g");
        store.seed("route-a@eostage:agent-model", "route-a", "fixture-key-a");

        assert_eq!(store.key("route-a").as_deref(), Some("fixture-key-a"));
        assert_eq!(store.targets(), [LEGACY, "eostage:agent-model:route-a"]);
        assert_eq!(
            store.entry(LEGACY),
            Some(("ghost".to_owned(), "fixture-key-g".to_owned()))
        );
        assert!(!store.decoded.borrow().iter().any(|target| target == LEGACY));
    }

    #[test]
    fn agent_model_saved_key_clears_the_legacy_entry_so_delete_is_final() {
        let store = FakeTargets::default();
        store.seed(LEGACY, "route-a", "fixture-key-old");

        store_with(&store, "route-a", "fixture-key-new").unwrap();
        assert_eq!(store.targets(), ["eostage:agent-model:route-a"]);
        assert!(delete_with(&store, "route-a").unwrap());
        assert!(!ready_with(&store, "route-a").unwrap());
        assert!(store.targets().is_empty());
    }

    #[test]
    fn agent_model_key_delete_removes_only_the_channel_legacy_entries_without_decoding() {
        let store = FakeTargets::default();
        store.seed(LEGACY, "route-a", "fixture-key-a");
        store.seed("route-b@eostage:agent-model", "route-b", "fixture-key-b");

        assert!(delete_with(&store, "route-a").unwrap());
        assert_eq!(store.targets(), ["route-b@eostage:agent-model"]);

        // The bare legacy target belongs to another channel: left alone.
        store.seed(LEGACY, "route-b", "fixture-key-b2");
        assert!(!delete_with(&store, "route-a").unwrap());
        assert_eq!(
            store.entry(LEGACY).map(|(owner, _)| owner).as_deref(),
            Some("route-b")
        );
        assert_eq!(store.decodes.get(), 0);
    }

    #[test]
    fn agent_model_key_events_never_carry_a_secret() {
        let store = FakeTargets::default();
        store.seed(LEGACY, "route-a", "fixture-secret-a");
        store.seed("route-b@eostage:agent-model", "route-b", "fixture-secret-b");
        store.seed("route-c.eostage:agent-model", "route-c", "fixture-secret-c");
        store.seed("eostage:agent-model:route-d", "route-d", "fixture-secret-d");
        store.seed(
            "route-d@eostage:agent-model",
            "route-d",
            "fixture-secret-d0",
        );

        for channel in ["route-a", "route-b", "route-c", "route-d"] {
            assert!(ready_with(&store, channel).unwrap());
        }
        // route-d's channel target already existed: its legacy entry stays
        // until the key is saved again.
        store_with(&store, "route-d", "fixture-secret-d2").unwrap();
        store_with(&store, "route-e", "fixture-secret-e").unwrap();
        delete_with(&store, "route-a").unwrap();

        let events = store.events();
        assert_eq!(events.len(), 4, "{events:?}");
        for event in &events {
            assert!(!event.contains("fixture-secret"), "{event}");
            assert!(
                event.starts_with("AGENT_MODEL_KEY_MIGRATED channel=route-")
                    || event.starts_with("AGENT_MODEL_KEY_LEGACY_REMOVED channel=route-"),
                "{event}"
            );
        }
    }

    /// Runs the real Windows exact-target calls against one isolated test
    /// target only (never a `eostage:` target), removed again on drop.
    #[test]
    #[cfg(target_os = "windows")]
    fn windows_owned_target_calls_skip_other_owners_and_delete_undecodable_values() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = format!("sceneport-test:agent-model:{}:{nonce}", std::process::id());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = platform_keyring::delete_owned(&self.0, "owner-x");
            }
        }
        let _cleanup = Cleanup(target.clone());
        // A lone UTF-16 high surrogate: neither UTF-16 nor UTF-8 decoding
        // accepts it.
        platform_keyring::write_raw_for_test(&target, "owner-x", &0xD800u16.to_le_bytes()).unwrap();

        assert!(matches!(
            platform_keyring::read_owned(&target, "owner-x"),
            Err(DesktopCredentialError::StoreUnavailable)
        ));
        assert!(matches!(
            platform_keyring::read_owned(&target, "owner-y"),
            Ok(None)
        ));
        assert!(matches!(
            platform_keyring::delete_owned(&target, "owner-y"),
            Ok(false)
        ));
        assert!(matches!(
            platform_keyring::delete_owned(&target, "owner-x"),
            Ok(true)
        ));
        assert!(matches!(
            platform_keyring::read_owned(&target, "owner-x"),
            Ok(None)
        ));
    }
}
