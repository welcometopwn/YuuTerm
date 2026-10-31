//! Shared Windows credential-store boundary for SSH, RDP and VNC passwords.
//! Only app-owned generic targets can be constructed. Secrets never cross IPC.
use sha2::{Digest, Sha256};
use windows::{
    Win32::{
        Foundation::{ERROR_CANCELLED, ERROR_LOGON_FAILURE, ERROR_NOT_FOUND, HWND},
        Security::Credentials::{
            CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PACK_GENERIC_CREDENTIALS,
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CREDUI_INFOW,
            CREDUIWIN_CHECKBOX, CREDUIWIN_GENERIC, CredDeleteW, CredFree,
            CredPackAuthenticationBufferW, CredReadW, CredUIPromptForWindowsCredentialsW,
            CredUnPackAuthenticationBufferW, CredWriteW,
        },
        System::Com::CoTaskMemFree,
    },
    core::{BOOL, HRESULT, PCWSTR, PWSTR},
};
use yuuterm_core::Profile;
use zeroize::{Zeroize, Zeroizing};

/// No Serialize/Clone, and Debug never reveals its contents. Zeroization is
/// best-effort process-memory hygiene, not protection against an infected user.
pub struct Secret(Zeroizing<Vec<u8>>);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self, String> {
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty()
            || bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize
            || std::str::from_utf8(&bytes).is_err()
        {
            return Err("Credential must contain 1–2560 UTF-8 bytes".into());
        }
        Ok(Self(bytes))
    }
    pub fn expose_for_authentication(&self) -> &str {
        // Validated once at construction, including data read back from Windows.
        std::str::from_utf8(&self.0).expect("Secret UTF-8 invariant")
    }
}

pub struct Target(String);
/// Non-secret, non-recycled local storage identity. Future import/copy code must
/// create a new value rather than transferring one from an exchange document.
pub struct StorageIdentity(String);
impl StorageIdentity {
    pub fn fresh() -> Result<Self, String> {
        let guid = unsafe { windows::Win32::System::Com::CoCreateGuid() }
            .map_err(|e| failure("identity creation", e))?;
        Self::from_local_metadata(&format!("{guid:?}"))
    }
    pub fn from_local_metadata(value: &str) -> Result<Self, String> {
        if value.len() != 36
            || !value.bytes().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
            || value.bytes().filter(|b| *b != b'-').all(|b| b == b'0')
        {
            return Err("Invalid local credential identity".into());
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
    pub fn as_local_metadata(&self) -> &str {
        &self.0
    }
}
impl Target {
    /// Bind to the bookmark AND endpoint/account/protocol. Editing a destination
    /// or importing/copying to a new ID must not silently reuse an old password.
    pub fn for_profile(profile: &Profile, identity: &StorageIdentity) -> Result<Self, String> {
        profile.validate()?;
        if profile.id == 0 {
            return Err("A saved session is required".into());
        }
        let mut metadata = serde_json::to_vec(&(
            1u32,
            &identity.0,
            profile.id,
            profile.protocol,
            profile.host.to_ascii_lowercase(),
            profile.port,
            &profile.username,
        ))
        .map_err(|_| "Cannot identify credential destination")?;
        // Keep existing SSH/RDP targets stable. VNC passwords must also be
        // isolated between account/password and password-only authentication.
        if profile.protocol == yuuterm_core::Protocol::Vnc {
            metadata.extend_from_slice(match profile.vnc.auth {
                yuuterm_core::vnc::VncAuth::Plain => b"/vnc/plain",
                yuuterm_core::vnc::VncAuth::Vnc => b"/vnc/password",
            });
        }
        Ok(Self(format!(
            "YuuTerm/password/v1/{:x}",
            Sha256::digest(metadata)
        )))
    }
    /// The saved-password target of a bookmark that already has a local identity.
    pub fn saved_for(profile: &Profile) -> Result<Option<Self>, String> {
        if profile.credential_id.is_empty() {
            return Ok(None);
        }
        let identity = StorageIdentity::from_local_metadata(&profile.credential_id)?;
        Self::for_profile(profile, &identity).map(Some)
    }
    fn wide(&self) -> Vec<u16> {
        self.0.encode_utf16().chain(Some(0)).collect()
    }
}

fn missing(error: &windows::core::Error) -> bool {
    error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
}
fn failure(operation: &str, error: windows::core::Error) -> String {
    // Include an OS status for diagnostics, not target names or secret buffers.
    format!(
        "Windows credential {operation} failed ({:#x})",
        error.code().0
    )
}

/// Own exactly one allocation returned by CredRead. Clear the returned blob
/// before CredFree; never keep the native pointer across an await or thread hop.
struct Owned(*mut CREDENTIALW);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            if let Some(credential) = self.0.as_mut() {
                if !credential.CredentialBlob.is_null()
                    && credential.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
                {
                    std::slice::from_raw_parts_mut(
                        credential.CredentialBlob,
                        credential.CredentialBlobSize as usize,
                    )
                    .zeroize();
                }
                CredFree(self.0.cast());
            }
        }
    }
}

/// Call on a blocking worker. Persistence is the current user on this computer,
/// despite the Win32 constant's LOCAL_MACHINE name; no credential roaming flag.
pub fn store(target: &Target, username: &str, secret: &Secret) -> Result<(), String> {
    if username.len() > 256 || username.chars().any(char::is_control) {
        return Err("Invalid credential account name".into());
    }
    let mut target = target.wide();
    let mut user: Vec<u16> = username.encode_utf16().chain(Some(0)).collect();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        UserName: PWSTR(user.as_mut_ptr()),
        CredentialBlobSize: secret.0.len() as u32,
        CredentialBlob: secret.0.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // CredWrite copies the input; vectors and immutable blob stay alive here.
    unsafe { CredWriteW(&credential, 0) }.map_err(|e| failure("write", e))
}

/// Returns the stored account name and secret.
pub fn load(target: &Target) -> Result<Option<(String, Secret)>, String> {
    let name = target.wide();
    let mut pointer = std::ptr::null_mut();
    if let Err(error) =
        unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut pointer) }
    {
        return if missing(&error) {
            Ok(None)
        } else {
            Err(failure("read", error))
        };
    }
    let allocation = Owned(pointer);
    let credential =
        unsafe { allocation.0.as_ref() }.ok_or("Windows returned an empty credential")?;
    if credential.Type != CRED_TYPE_GENERIC
        || credential.CredentialBlobSize == 0
        || credential.CredentialBlobSize > CRED_MAX_CREDENTIAL_BLOB_SIZE
        || credential.CredentialBlob.is_null()
    {
        return Err("Windows returned an invalid credential blob".into());
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    let user = if credential.UserName.is_null() {
        String::new()
    } else {
        unsafe { credential.UserName.to_string() }.unwrap_or_default()
    };
    Ok(Some((user, Secret::new(bytes.to_vec())?)))
}

/// Bookmarks whose saved password was just refused; the next prompt says so.
static REFUSED: std::sync::Mutex<std::collections::BTreeSet<u64>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());
pub fn mark_refused(profile_id: u64) {
    if let Ok(mut set) = REFUSED.lock() {
        set.insert(profile_id);
    }
}
fn take_refused(profile_id: u64) -> bool {
    REFUSED.lock().is_ok_and(|mut set| set.remove(&profile_id))
}

/// Credentials for one connection: the saved password, or the native prompt.
pub struct Resolved {
    pub username: String,
    pub secret: Secret,
    /// The saved password used or just stored; forget it if it is refused.
    pub saved: Option<Target>,
    /// Identity to persist in the bookmark when a password was saved first.
    pub new_identity: Option<String>,
}

/// Saved password of `profile`, otherwise the native prompt (modal; call on the
/// UI thread WITHOUT holding application locks, other IPC runs inside it). A
/// remembered password is stored under the bookmark's identity. `None` when
/// the user cancels.
pub fn resolve(
    parent: HWND,
    profile: &Profile,
    caption: &str,
    default_port: u16,
) -> Result<Option<Resolved>, String> {
    resolve_for_account(parent, profile, caption, default_port, true)
}

/// Shared resolver, including password-only VNC. Call only after the protocol's
/// trust gate; saved credentials never bypass server identity validation.
pub fn resolve_for_account(
    parent: HWND,
    profile: &Profile,
    caption: &str,
    default_port: u16,
    account: bool,
) -> Result<Option<Resolved>, String> {
    if !profile.temporary
        && let Some(target) = Target::saved_for(profile)?
        && let Some((user, secret)) = load(&target)?
    {
        return Ok(Some(Resolved {
            username: if !account {
                String::new()
            } else if user.is_empty() {
                profile.username.clone()
            } else {
                user
            },
            secret,
            saved: Some(target),
            new_identity: None,
        }));
    }
    let retry = take_refused(profile.id);
    let message = format!(
        "Sign in to {}{}",
        profile.host,
        if profile.port == default_port {
            String::new()
        } else {
            format!(":{}", profile.port)
        }
    );
    // Quick-connect sessions are not saved, so neither is their password.
    let prompted = if profile.temporary {
        prompt_once(
            parent,
            caption,
            &message,
            if account { &profile.username } else { "" },
            account,
        )?
    } else {
        prompt_with(
            parent,
            caption,
            &message,
            if account { &profile.username } else { "" },
            retry,
            true,
            account,
        )?
    };
    let Some(prompted) = prompted else {
        return Ok(None);
    };
    let mut result = Resolved {
        username: prompted.username,
        secret: prompted.secret,
        saved: None,
        new_identity: None,
    };
    if profile.protocol == yuuterm_core::Protocol::Vnc && !account {
        let password = result.secret.expose_for_authentication();
        if !password.is_ascii() || password.len() > 8 {
            return Err("VNC requires an ASCII password of at most eight characters".into());
        }
        result.username.clear();
    }
    if prompted.save {
        let identity = if profile.credential_id.is_empty() {
            let fresh = StorageIdentity::fresh()?;
            result.new_identity = Some(fresh.as_local_metadata().to_owned());
            fresh
        } else {
            StorageIdentity::from_local_metadata(&profile.credential_id)?
        };
        let target = Target::for_profile(profile, &identity)?;
        store(&target, &result.username, &result.secret)?;
        result.saved = Some(target);
    }
    Ok(Some(result))
}

/// Result of the native Windows credential prompt.
pub struct Prompted {
    pub username: String,
    pub secret: Secret,
    pub save: bool,
}

/// Native Windows Security prompt (CredUI) owned by `parent`, with a "Remember
/// me" checkbox. Modal; must run on the parent's UI thread. The password never
/// reaches the WebView. `None` when the user cancels.
pub fn prompt(
    parent: HWND,
    caption: &str,
    message: &str,
    username: &str,
    retry: bool,
) -> Result<Option<Prompted>, String> {
    prompt_with(parent, caption, message, username, retry, true, true)
}

/// The same native prompt without "Remember me", for secrets YuuTerm must not
/// save (unsaved quick connections). With `account` false an empty user name is accepted and the
/// password alone is used. `save` is always false in the result.
pub fn prompt_once(
    parent: HWND,
    caption: &str,
    message: &str,
    username: &str,
    account: bool,
) -> Result<Option<Prompted>, String> {
    prompt_with(parent, caption, message, username, false, false, account)
}

fn prompt_with(
    parent: HWND,
    caption: &str,
    message: &str,
    username: &str,
    retry: bool,
    remember: bool,
    account: bool,
) -> Result<Option<Prompted>, String> {
    let caption: Vec<u16> = caption.encode_utf16().chain(Some(0)).collect();
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let info = CREDUI_INFOW {
        cbSize: size_of::<CREDUI_INFOW>() as u32,
        hwndParent: parent,
        pszMessageText: PCWSTR(message.as_ptr()),
        pszCaptionText: PCWSTR(caption.as_ptr()),
        ..Default::default()
    };
    // Prefill the account name only.
    let user: Vec<u16> = username.encode_utf16().chain(Some(0)).collect();
    let empty = [0u16];
    let mut packed = Vec::new();
    if !username.is_empty() {
        let mut size = 0u32;
        let _ = unsafe {
            CredPackAuthenticationBufferW(
                CRED_PACK_GENERIC_CREDENTIALS,
                PCWSTR(user.as_ptr()),
                PCWSTR(empty.as_ptr()),
                None,
                &mut size,
            )
        };
        if size > 0 && size < 65536 {
            packed = vec![0u8; size as usize];
            if unsafe {
                CredPackAuthenticationBufferW(
                    CRED_PACK_GENERIC_CREDENTIALS,
                    PCWSTR(user.as_ptr()),
                    PCWSTR(empty.as_ptr()),
                    Some(packed.as_mut_ptr()),
                    &mut size,
                )
            }
            .is_err()
            {
                packed.clear();
            }
        }
    }
    let mut package = 0u32;
    let mut out = std::ptr::null_mut();
    let mut out_size = 0u32;
    let mut save = BOOL(0);
    let status = unsafe {
        CredUIPromptForWindowsCredentialsW(
            Some(&info),
            if retry { ERROR_LOGON_FAILURE.0 } else { 0 },
            &mut package,
            (!packed.is_empty()).then_some(packed.as_ptr().cast()),
            packed.len() as u32,
            &mut out,
            &mut out_size,
            remember.then_some(&mut save),
            if remember {
                CREDUIWIN_GENERIC | CREDUIWIN_CHECKBOX
            } else {
                CREDUIWIN_GENERIC
            },
        )
    };
    if status == ERROR_CANCELLED.0 {
        return Ok(None);
    }
    if status != 0 || out.is_null() {
        return Err(format!("Windows credential prompt failed ({status:#x})"));
    }
    // Own the CoTaskMem buffer: clear it before freeing.
    struct Out(*mut std::ffi::c_void, u32);
    impl Drop for Out {
        fn drop(&mut self) {
            unsafe {
                std::slice::from_raw_parts_mut(self.0 as *mut u8, self.1 as usize).zeroize();
                CoTaskMemFree(Some(self.0));
            }
        }
    }
    let out = Out(out, out_size);
    let mut user = Zeroizing::new(vec![0u16; 514]);
    let mut domain = Zeroizing::new(vec![0u16; 338]);
    let mut password = Zeroizing::new(vec![0u16; 1281]);
    let (mut user_len, mut domain_len, mut password_len) = (
        user.len() as u32,
        domain.len() as u32,
        password.len() as u32,
    );
    unsafe {
        CredUnPackAuthenticationBufferW(
            CRED_PACK_GENERIC_CREDENTIALS,
            out.0,
            out.1,
            Some(PWSTR(user.as_mut_ptr())),
            &mut user_len,
            Some(PWSTR(domain.as_mut_ptr())),
            Some(&mut domain_len),
            Some(PWSTR(password.as_mut_ptr())),
            &mut password_len,
        )
    }
    .map_err(|e| failure("prompt result", e))?;
    let text = |buffer: &[u16]| {
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        String::from_utf16(&buffer[..end]).map_err(|_| "Invalid credential text".to_string())
    };
    let username = text(&user)?;
    let password = Zeroizing::new(text(&password)?);
    if username.len() > 256
        || username.chars().any(char::is_control)
        || (account && username.trim().is_empty())
    {
        return Err("Enter a user name".into());
    }
    let secret = Secret::new(password.as_bytes().to_vec())?;
    Ok(Some(Prompted {
        username,
        secret,
        save: remember && save.as_bool(),
    }))
}

pub fn forget(target: &Target) -> Result<(), String> {
    let name = target.wide();
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(e) if missing(&e) => Ok(()),
        Err(e) => Err(failure("delete", e)),
    }
}

/// Changes that must not inherit a saved password. VNC authentication modes
/// use separate credentials even when the destination and account are equal.
pub fn same_destination(a: &Profile, b: &Profile) -> bool {
    a.protocol == b.protocol
        && a.host.eq_ignore_ascii_case(&b.host)
        && a.port == b.port
        && a.username == b.username
        && (a.protocol != yuuterm_core::Protocol::Vnc || a.vnc.auth == b.vnc.auth)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        Profile {
            id: 7,
            name: "Credential fixture".into(),
            host: "example.test".into(),
            username: "fixture".into(),
            ..Default::default()
        }
    }
    #[test]
    fn targets_do_not_reuse_secrets_after_destination_or_identity_changes() {
        let p = profile();
        let identity = StorageIdentity::fresh().unwrap();
        let baseline = Target::for_profile(&p, &identity).unwrap();
        for variant in 0..5 {
            let mut changed = p.clone();
            match variant {
                0 => changed.id += 1,
                1 => changed.host = "other.test".into(),
                2 => changed.port += 1,
                3 => changed.username = "other".into(),
                _ => changed.protocol = yuuterm_core::Protocol::Rdp,
            }
            assert_ne!(
                Target::for_profile(&changed, &identity).unwrap().0,
                baseline.0
            );
        }
        let mut renamed = p.clone();
        renamed.name = "Renamed".into();
        renamed.host = "EXAMPLE.TEST".into();
        assert_eq!(
            Target::for_profile(&renamed, &identity).unwrap().0,
            baseline.0
        );
        let restored = StorageIdentity::from_local_metadata(identity.as_local_metadata()).unwrap();
        assert_eq!(Target::for_profile(&p, &restored).unwrap().0, baseline.0);
        let recycled = StorageIdentity::fresh().unwrap();
        assert_ne!(
            Target::for_profile(&p, &recycled).unwrap().0,
            baseline.0,
            "Reusing a numeric bookmark ID must not recover a deleted bookmark's password"
        );
        assert!(
            StorageIdentity::from_local_metadata("00000000-0000-0000-0000-000000000000").is_err()
        );
        assert!(baseline.0.starts_with("YuuTerm/password/v1/"));
        assert!(!baseline.0.contains("example.test"));
    }
    #[test]
    fn secret_validation_and_debug_never_reflect_input() {
        let s = Secret::new(b"not-a-real-password".to_vec()).unwrap();
        assert_eq!(format!("{s:?}"), "Secret([redacted])");
        for bad in [Vec::new(), vec![255], vec![b'x'; 2561]] {
            assert!(Secret::new(bad).is_err());
        }
    }
    #[test]
    fn vnc_modes_have_separate_targets_and_use_shared_saved_resolver() {
        let identity = StorageIdentity::fresh().unwrap();
        let mut p = profile();
        p.protocol = yuuterm_core::Protocol::Vnc;
        p.credential_id = identity.as_local_metadata().into();
        let plain = Target::saved_for(&p).unwrap().unwrap();
        let mut other = p.clone();
        other.vnc.auth = yuuterm_core::vnc::VncAuth::Vnc;
        assert!(!same_destination(&p, &other));
        assert_ne!(plain.0, Target::saved_for(&other).unwrap().unwrap().0);
        other.name = "Renamed".into();
        assert!(same_destination(&other, &other.clone()));
        struct Cleanup(Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = forget(&self.0);
            }
        }
        for (profile, account) in [(&p, true), (&other, false)] {
            let target = Cleanup(Target::saved_for(profile).unwrap().unwrap());
            assert!(load(&target.0).unwrap().is_none());
            store(
                &target.0,
                if account { "fixture" } else { "" },
                &Secret::new(b"password".to_vec()).unwrap(),
            )
            .unwrap();
            // Null parent is safe: a saved credential must not open any dialog.
            let resolved =
                resolve_for_account(HWND::default(), profile, "Never shown", 5900, account)
                    .unwrap()
                    .unwrap();
            assert_eq!(resolved.username, if account { "fixture" } else { "" });
            assert_eq!(resolved.secret.expose_for_authentication(), "password");
            assert!(resolved.saved.is_some() && resolved.new_identity.is_none());
        }
    }
    #[test]
    fn actual_windows_store_roundtrip_replace_and_delete_are_isolated() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = Target(format!(
            "YuuTerm/developer-test/v1/{}-{nonce}",
            std::process::id()
        ));
        // Refuse to touch an existing entry, even in the test namespace.
        assert!(load(&target).unwrap().is_none());
        struct Cleanup(Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = forget(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        let first = Secret::new("dummy-only-秘密-€".as_bytes().to_vec()).unwrap();
        store(&cleanup.0, "fixture", &first).unwrap();
        let (user, loaded) = load(&cleanup.0).unwrap().unwrap();
        assert_eq!(user, "fixture");
        assert!(loaded.expose_for_authentication() == first.expose_for_authentication());
        let replacement = Secret::new(b"dummy-replacement".to_vec()).unwrap();
        store(&cleanup.0, "fixture", &replacement).unwrap();
        assert!(
            load(&cleanup.0)
                .unwrap()
                .unwrap()
                .1
                .expose_for_authentication()
                == replacement.expose_for_authentication()
        );
        forget(&cleanup.0).unwrap();
        assert!(load(&cleanup.0).unwrap().is_none());
        forget(&cleanup.0).unwrap();
    }
}
