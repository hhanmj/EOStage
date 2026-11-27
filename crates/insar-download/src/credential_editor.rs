//! Exact local field edits for the explicit account Settings surface.
//! Login validation stays at the existing authentication boundary. Neither
//! partial edits nor successful persistence imply authentication.
use super::*;
use zeroize::Zeroizing;

#[cfg(target_os = "windows")]
fn read_editor_target(service: &str, key: &str) -> Result<Option<String>, DesktopCredentialError> {
    platform_keyring::get(service, key)
}
#[cfg(target_os = "macos")]
fn read_editor_target(service: &str, key: &str) -> Result<Option<String>, DesktopCredentialError> {
    platform_keyring::get_raw(service, key)
}
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn read_editor_target(service: &str, key: &str) -> Result<Option<String>, DesktopCredentialError> {
    match keyring::Entry::new(service, key)
        .map_err(|_| DesktopCredentialError::StoreUnavailable)?
        .get_password()
    {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(DesktopCredentialError::StoreUnavailable),
    }
}

fn validate_edit(value: &str) -> Result<(), DesktopCredentialError> {
    // Windows CredentialBlob is UTF-16 and bounded to 2560 bytes. Check before
    // any store I/O; never truncate a password or reflect it in an error.
    if value.encode_utf16().count() > 1280 || value.chars().any(char::is_control) {
        return Err(DesktopCredentialError::InvalidInput);
    }
    Ok(())
}

pub fn read_desktop_credential_for_edit(
    field: DesktopCredentialField,
) -> Result<Option<Zeroizing<String>>, DesktopCredentialError> {
    // Only Settings entry calls this: status/focus refresh never returns values.
    // Do not import environment or .netrc values into editable application data.
    let _guard = if matches!(
        field,
        DesktopCredentialField::BhoonidhiUsername | DesktopCredentialField::BhoonidhiPassword
    ) {
        bhoonidhi_credential_read_guard().map_err(|_| DesktopCredentialError::StoreUnavailable)?
    } else {
        desktop_login_pair_read_guard()?
    };
    let (service, key) = field.target();
    read_editor_target(service, key).map(|value| value.map(Zeroizing::new))
}

fn apply_edit(
    field: DesktopCredentialField,
    value: &str,
    get: impl Fn(&str, &str) -> Result<Option<String>, DesktopCredentialError>,
    set: impl Fn(&str, &str, &str) -> Result<(), DesktopCredentialError>,
    delete: impl Fn(&str, &str) -> Result<bool, DesktopCredentialError>,
) -> Result<bool, DesktopCredentialError> {
    validate_edit(value)?;
    let (service, key) = field.target();
    let mut changes = vec![(key, (!value.is_empty()).then_some(value))];
    // Preserve the historical explicit Earthdata mode switch. Empty edits only
    // clear that field; they never delete a different, still usable credential.
    if !value.is_empty() {
        match field {
            DesktopCredentialField::EarthdataToken => changes.extend([
                (EARTHDATA_USERNAME_KEY, None),
                (EARTHDATA_PASSWORD_KEY, None),
            ]),
            DesktopCredentialField::EarthdataUsername
            | DesktopCredentialField::EarthdataPassword => {
                changes.push((EARTHDATA_TOKEN_KEY, None))
            }
            _ => (),
        }
    }
    let previous = changes
        .iter()
        .map(|(key, _)| get(service, key).map(|value| value.map(Zeroizing::new)))
        .collect::<Result<Vec<_>, _>>()?;
    if changes
        .iter()
        .zip(&previous)
        .all(|((_, next), prior)| *next == prior.as_deref().map(String::as_str))
    {
        return Ok(false);
    }
    for (index, (key, next)) in changes.iter().enumerate() {
        let result = match next {
            Some(value) => set(service, key, value),
            None => delete(service, key).map(|_| ()),
        };
        if let Err(error) = result {
            // Restore every touched key, including one whose failed API call
            // may have partially succeeded. Report failure even after rollback.
            let mut rollback_failed = false;
            for ((key, _), old) in changes[..=index].iter().zip(&previous) {
                let restored = match old {
                    Some(value) => set(service, key, value),
                    None => delete(service, key).map(|_| ()),
                };
                rollback_failed |= restored.is_err();
            }
            return Err(if rollback_failed {
                DesktopCredentialError::StoreUnavailable
            } else {
                error
            });
        }
    }
    Ok(true)
}

pub fn update_desktop_credential_field(
    field: DesktopCredentialField,
    value: String,
) -> Result<(), DesktopCredentialError> {
    let value = Zeroizing::new(value);
    validate_edit(&value)?;
    let bhoonidhi = matches!(
        field,
        DesktopCredentialField::BhoonidhiUsername | DesktopCredentialField::BhoonidhiPassword
    );
    let _guard = if bhoonidhi {
        bhoonidhi_credential_write_guard()?
    } else {
        desktop_login_pair_write_guard()?
    };
    let changed = apply_edit(
        field,
        &value,
        read_editor_target,
        platform_keyring::set,
        platform_keyring::delete,
    )?;
    if changed && bhoonidhi {
        mark_bhoonidhi_credential_revision()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Store {
        values: RefCell<BTreeMap<(String, String), String>>,
        fail_once: RefCell<Option<String>>,
    }
    impl Store {
        fn get(&self, service: &str, key: &str) -> Result<Option<String>, DesktopCredentialError> {
            Ok(self
                .values
                .borrow()
                .get(&(service.into(), key.into()))
                .cloned())
        }
        fn set(&self, service: &str, key: &str, value: &str) -> Result<(), DesktopCredentialError> {
            if self.fail_once.borrow().as_deref() == Some(key) {
                self.fail_once.replace(None);
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            self.values
                .borrow_mut()
                .insert((service.into(), key.into()), value.into());
            Ok(())
        }
        fn delete(&self, service: &str, key: &str) -> Result<bool, DesktopCredentialError> {
            if self.fail_once.borrow().as_deref() == Some(key) {
                self.fail_once.replace(None);
                return Err(DesktopCredentialError::StoreUnavailable);
            }
            Ok(self
                .values
                .borrow_mut()
                .remove(&(service.into(), key.into()))
                .is_some())
        }
        fn edit(
            &self,
            field: DesktopCredentialField,
            value: &str,
        ) -> Result<bool, DesktopCredentialError> {
            apply_edit(
                field,
                value,
                |s, k| self.get(s, k),
                |s, k, v| self.set(s, k, v),
                |s, k| self.delete(s, k),
            )
        }
    }

    #[test]
    fn partial_unicode_and_empty_edits_preserve_exact_values_and_other_fields() {
        let store = Store::default();
        for (user, secret) in [
            (
                DesktopCredentialField::CdseUsername,
                DesktopCredentialField::CdsePassword,
            ),
            (
                DesktopCredentialField::UsgsUsername,
                DesktopCredentialField::UsgsApplicationToken,
            ),
            (
                DesktopCredentialField::BhoonidhiUsername,
                DesktopCredentialField::BhoonidhiPassword,
            ),
        ] {
            store.edit(user, "测").unwrap();
            store.edit(user, "测试 a ").unwrap();
            store.edit(secret, "partial secret ").unwrap();
            store.edit(user, "").unwrap();
            let (s, k) = secret.target();
            assert_eq!(store.get(s, k).unwrap().as_deref(), Some("partial secret "));
            let (s, k) = user.target();
            assert!(store.get(s, k).unwrap().is_none());
        }
        store
            .edit(DesktopCredentialField::GacosEmail, "not-yet-an-email")
            .unwrap();
        store
            .edit(DesktopCredentialField::OpentopographyKey, " ")
            .unwrap();
        assert_eq!(
            store
                .get(OPENTOPOGRAPHY_KEYRING_SERVICE, OPENTOPOGRAPHY_KEY)
                .unwrap()
                .as_deref(),
            Some(" ")
        );
    }

    #[test]
    fn earthdata_switch_is_atomic_and_failed_storage_never_claims_success() {
        let store = Store::default();
        store
            .edit(DesktopCredentialField::EarthdataUsername, "user")
            .unwrap();
        store
            .edit(DesktopCredentialField::EarthdataPassword, "old")
            .unwrap();
        let before = store.values.borrow().clone();
        store.fail_once.replace(Some(EARTHDATA_PASSWORD_KEY.into()));
        assert!(store
            .edit(DesktopCredentialField::EarthdataToken, "new")
            .is_err());
        assert_eq!(*store.values.borrow(), before);
        store
            .edit(DesktopCredentialField::EarthdataToken, "new")
            .unwrap();
        assert_eq!(store.values.borrow().len(), 1);
        assert!(!store
            .edit(DesktopCredentialField::EarthdataToken, "new")
            .unwrap());
        store
            .edit(DesktopCredentialField::EarthdataToken, "")
            .unwrap();
        assert!(store.values.borrow().is_empty());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_editor_roundtrip_uses_only_an_isolated_test_target_and_reopens_in_a_child() {
        const PREFIX: &str = "sceneport-test:credential-editor:";
        const CHILD: &str = "EOSTAGE_TEST_EDITOR_TARGET";
        if let Ok(target) = std::env::var(CHILD) {
            assert!(target.starts_with(PREFIX));
            assert_eq!(
                read_editor_target(&target, CDSE_USERNAME_KEY)
                    .unwrap()
                    .as_deref(),
                Some("测试 user ")
            );
            assert_eq!(
                read_editor_target(&target, CDSE_PASSWORD_KEY)
                    .unwrap()
                    .as_deref(),
                Some("synthetic secret ")
            );
            return;
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = format!("{PREFIX}{}:{nonce}", std::process::id());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for key in [CDSE_USERNAME_KEY, CDSE_PASSWORD_KEY] {
                    let _ = platform_keyring::delete(&self.0, key);
                }
            }
        }
        let _cleanup = Cleanup(target.clone());
        let edit = |field, value| {
            apply_edit(
                field,
                value,
                |_, key| read_editor_target(&target, key),
                |_, key, value| platform_keyring::set(&target, key, value),
                |_, key| platform_keyring::delete(&target, key),
            )
        };
        edit(DesktopCredentialField::CdseUsername, "测").unwrap();
        edit(DesktopCredentialField::CdsePassword, "synthetic secret ").unwrap();
        edit(DesktopCredentialField::CdseUsername, "测试 user ").unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "credentials::editor::tests::windows_editor_roundtrip_uses_only_an_isolated_test_target_and_reopens_in_a_child", "--test-threads=1"])
            .env(CHILD, &target).output().unwrap();
        assert!(
            child.status.success(),
            "isolated child credential verification failed"
        );
        edit(DesktopCredentialField::CdseUsername, "").unwrap();
        assert!(read_editor_target(&target, CDSE_USERNAME_KEY)
            .unwrap()
            .is_none());
        assert_eq!(
            read_editor_target(&target, CDSE_PASSWORD_KEY)
                .unwrap()
                .as_deref(),
            Some("synthetic secret ")
        );
        edit(DesktopCredentialField::CdsePassword, "").unwrap();
        assert!(read_editor_target(&target, CDSE_PASSWORD_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn unsafe_or_oversized_fields_fail_before_any_store_access() {
        for value in [
            "bad\0value".to_owned(),
            "a\nb".into(),
            "x".repeat(1281),
            "😀".repeat(641),
        ] {
            assert!(apply_edit(
                DesktopCredentialField::CdsePassword,
                &value,
                |_, _| panic!("no read"),
                |_, _, _| panic!("no write"),
                |_, _| panic!("no delete")
            )
            .is_err());
        }
    }
}
