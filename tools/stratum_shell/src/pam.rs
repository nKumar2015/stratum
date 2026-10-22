//! Password checks through PAM.
//!
//! A minimal binding to the four calls a lock screen needs: start a
//! transaction, authenticate, check the account, and end. The conversation
//! answers every prompt with the typed password, which is what swaylock and
//! hyprlock do, and collects error and info messages so the lock screen can
//! show why an attempt failed when PAM gives a reason.
//!
//! Authentication blocks (pam_unix sleeps after a failure), so callers run it
//! on a worker thread.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_BUF_ERR: c_int = 5;
const PAM_AUTH_ERR: c_int = 7;
const PAM_CONV_ERR: c_int = 19;
const PAM_REFRESH_CRED: c_int = 0x0010;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: ConvFn,
    appdata_ptr: *mut c_void,
}

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_start_confdir(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        confdir: *const c_char,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(handle: *mut PamHandle, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut PamHandle, errnum: c_int) -> *const c_char;
}

/// A password that is wiped from memory when dropped.
///
/// The buffer is allocated with spare capacity so typing does not reallocate
/// and leave stale copies behind.
pub struct Secret(String);

impl Secret {
    pub fn new() -> Self {
        Self(String::with_capacity(256))
    }

    pub fn push(&mut self, ch: char) {
        // Refuse to grow past the reserved buffer rather than reallocate.
        if self.0.len() + ch.len_utf8() <= self.0.capacity() {
            self.0.push(ch);
        }
    }

    pub fn pop(&mut self) {
        if let Some(ch) = self.0.pop() {
            // `pop` leaves the removed bytes in the buffer's spare capacity.
            // SAFETY: only spare capacity is written, which the string's UTF-8
            // invariant does not cover.
            unsafe {
                let spare = self.0.as_mut_vec().spare_capacity_mut();
                for byte in spare.iter_mut().take(ch.len_utf8()) {
                    byte.write(0);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.wipe();
    }

    /// Number of characters, for drawing one dot per character.
    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Move the contents out, leaving this buffer empty.
    pub fn take(&mut self) -> Secret {
        let mut taken = Secret::new();
        for ch in self.0.chars() {
            taken.push(ch);
        }
        self.wipe();
        taken
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn wipe(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, and the length is reset to 0
        // straight after.
        unsafe {
            let bytes = self.0.as_mut_vec();
            bytes.iter_mut().for_each(|byte| *byte = 0);
            bytes.set_len(0);
        }
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// Why an attempt failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthError {
    /// A message from PAM itself when it gave one ("Account locked..."),
    /// otherwise PAM's description of the error code.
    pub message: String,
    /// The plain wrong-password case (`PAM_AUTH_ERR` with nothing more said),
    /// decided by the return code because PAM's messages are translated.
    pub wrong_password: bool,
}

/// State shared with the conversation callback.
struct Conversation {
    password: CString,
    messages: Vec<String>,
}

impl Drop for Conversation {
    fn drop(&mut self) {
        let mut password = std::mem::take(&mut self.password).into_bytes_with_nul();
        password.iter_mut().for_each(|byte| *byte = 0);
    }
}

unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    let conversation = &mut *(appdata as *mut Conversation);
    let count = num_msg as usize;
    // PAM frees the responses with free(), so they must come from malloc.
    let responses =
        libc::calloc(count, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }

    for index in 0..count {
        // Linux-PAM passes an array of pointers to messages.
        let message = *msg.add(index);
        if message.is_null() {
            continue;
        }
        let style = (*message).msg_style;
        let text = if (*message).msg.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*message).msg).to_string_lossy().into_owned()
        };
        match style {
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                let copy = libc::strdup(conversation.password.as_ptr());
                if copy.is_null() {
                    free_responses(responses, index);
                    return PAM_BUF_ERR;
                }
                (*responses.add(index)).resp = copy;
            }
            PAM_ERROR_MSG | PAM_TEXT_INFO => {
                if !text.trim().is_empty() {
                    conversation.messages.push(text.trim().to_string());
                }
            }
            _ => {
                free_responses(responses, index);
                return PAM_CONV_ERR;
            }
        }
    }

    *resp = responses;
    PAM_SUCCESS
}

/// Release responses already written when the conversation fails part-way.
unsafe fn free_responses(responses: *mut PamResponse, filled: usize) {
    for index in 0..filled {
        let resp = (*responses.add(index)).resp;
        if !resp.is_null() {
            let len = libc::strlen(resp);
            std::ptr::write_bytes(resp, 0, len);
            libc::free(resp as *mut c_void);
        }
    }
    libc::free(responses as *mut c_void);
}

/// Check `password` for `user` against the PAM `service`.
///
/// `confdir` points PAM at a directory other than `/etc/pam.d`; it exists so
/// tests can use a service whose outcome they control.
pub fn authenticate(
    service: &str,
    user: &str,
    password: Secret,
    confdir: Option<&Path>,
) -> Result<(), AuthError> {
    let fail = |message: &str| AuthError {
        message: message.to_string(),
        wrong_password: false,
    };
    let service = CString::new(service).map_err(|_| fail("invalid PAM service name"))?;
    let user = CString::new(user).map_err(|_| fail("invalid user name"))?;
    // `password` (the caller's buffer) is wiped when it drops at the end of
    // this function; the copy PAM sees is wiped by `Conversation`'s drop.
    let password = CString::new(password.as_str()).map_err(|_| fail("invalid password"))?;

    let mut conversation = Box::new(Conversation {
        password,
        messages: Vec::new(),
    });
    let conv = PamConv {
        conv: converse,
        appdata_ptr: &mut *conversation as *mut Conversation as *mut c_void,
    };

    let mut handle: *mut PamHandle = std::ptr::null_mut();
    // SAFETY: every pointer passed lives until `pam_end` below.
    let started = unsafe {
        match confdir {
            Some(dir) => {
                let dir = CString::new(dir.as_os_str().as_encoded_bytes())
                    .map_err(|_| fail("invalid PAM config directory"))?;
                pam_start_confdir(service.as_ptr(), user.as_ptr(), &conv, dir.as_ptr(), &mut handle)
            }
            None => pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle),
        }
    };
    if started != PAM_SUCCESS || handle.is_null() {
        return Err(fail("could not start PAM"));
    }

    // SAFETY: `handle` is a live PAM handle until `pam_end`.
    let status = unsafe {
        let mut status = pam_authenticate(handle, 0);
        if status == PAM_SUCCESS {
            status = pam_acct_mgmt(handle, 0);
        }
        if status == PAM_SUCCESS {
            // Lets modules such as gnome-keyring refresh what they hold. A
            // failure here does not mean the password was wrong.
            let _ = pam_setcred(handle, PAM_REFRESH_CRED);
        }
        status
    };

    let result = if status == PAM_SUCCESS {
        Ok(())
    } else {
        let wrong_password = status == PAM_AUTH_ERR && conversation.messages.is_empty();
        let message = conversation.messages.last().cloned().unwrap_or_else(|| {
            // SAFETY: pam_strerror returns a static string for the handle.
            unsafe {
                let text = pam_strerror(handle, status);
                if text.is_null() {
                    "authentication failed".to_string()
                } else {
                    CStr::from_ptr(text).to_string_lossy().into_owned()
                }
            }
        });
        Err(AuthError {
            message,
            wrong_password,
        })
    };

    // SAFETY: ends the transaction started above; the handle is not used again.
    unsafe {
        pam_end(handle, status);
    }
    drop(conversation);
    result
}

/// The PAM service to use, or `None` when no usable service exists.
///
/// PAM silently falls back to the `other` service when the named one has no
/// file, and on NixOS `other` denies everything. Locking with such a service
/// could never be undone, so only services that exist are considered: the
/// configured one first, then the shell's own, then Quickshell's (which the
/// QML lock screen used), then `login`, which every system has.
pub fn resolve_service(configured: &str, confdir: Option<&Path>) -> Option<String> {
    let dir: PathBuf = confdir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/etc/pam.d"));
    let candidates = [configured, "stratum-shell", "quickshell", "login"];
    candidates
        .iter()
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .find(|name| dir.join(name).is_file())
        .map(|name| name.to_string())
}

/// Check that this process can load every module the service's `auth` and
/// `account` stacks name.
///
/// PAM loads modules into the calling process, so they must be built against
/// a compatible libc. A shell built from an older nixpkgs than the system's
/// links an older glibc, and the system's `pam_unix.so` then fails to load
/// with "version GLIBC_x not found". PAM reports that only as a failed
/// attempt, so the lock would accept no password at all. Checking before
/// locking turns that into a refusal to lock.
///
/// Absolute module paths (what NixOS writes) are loaded; relative names are
/// resolved by PAM against its own directory and are skipped. `include`,
/// `substack`, and `@include` are followed.
pub fn check_service(service: &str, confdir: Option<&Path>) -> Result<(), String> {
    let dir: PathBuf = confdir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/etc/pam.d"));
    check_service_in(&dir, service, 0)
}

fn check_service_in(dir: &Path, service: &str, depth: usize) -> Result<(), String> {
    if depth > 8 || service.contains('/') {
        return Err(format!("PAM service '{service}' includes too deeply"));
    }
    let path = dir.join(service);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tokens = line.split_whitespace();
        let Some(kind) = tokens.next() else {
            continue;
        };
        if kind == "@include" {
            if let Some(name) = tokens.next() {
                check_service_in(dir, name, depth + 1)?;
            }
            continue;
        }
        // A leading '-' means "skip this module if it is missing".
        if kind.starts_with('-') || !matches!(kind, "auth" | "account") {
            continue;
        }
        let Some(mut control) = tokens.next().map(str::to_string) else {
            continue;
        };
        // Bracketed controls (`[success=1 default=ignore]`) span tokens.
        while control.starts_with('[') && !control.ends_with(']') {
            let Some(next) = tokens.next() else { break };
            control.push(' ');
            control.push_str(next);
        }
        let Some(module) = tokens.next() else {
            continue;
        };
        if matches!(control.as_str(), "include" | "substack") {
            check_service_in(dir, module, depth + 1)?;
        } else if module.starts_with('/') {
            load_module(module)?;
        }
    }
    Ok(())
}

fn load_module(path: &str) -> Result<(), String> {
    let c_path = CString::new(path).map_err(|_| format!("bad module path {path}"))?;
    // SAFETY: dlopen/dlclose on a NUL-terminated path; the handle is closed
    // straight away and no symbol from it is used.
    unsafe {
        let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let error = libc::dlerror();
            let reason = if error.is_null() {
                "unknown error".to_string()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            };
            return Err(format!("cannot load {path}: {reason}"));
        }
        libc::dlclose(handle);
    }
    Ok(())
}

/// Name of the user running the shell, from the password database.
pub fn current_user() -> Option<String> {
    // SAFETY: getpwuid_r writes into the buffers given; nothing is retained.
    unsafe {
        let uid = libc::getuid();
        let mut record: libc::passwd = std::mem::zeroed();
        let mut buffer = vec![0 as c_char; 4096];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let status = libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        );
        if status != 0 || result.is_null() || record.pw_name.is_null() {
            return std::env::var("USER").ok();
        }
        Some(CStr::from_ptr(record.pw_name).to_string_lossy().into_owned())
    }
}

/// A PAM config directory for tests and for exercising the lock screen
/// without the real password. Honoured only in debug builds, so a release
/// shell always authenticates against `/etc/pam.d`.
pub fn test_confdir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("STRATUM_PAM_CONFDIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> Secret {
        let mut secret = Secret::new();
        for ch in text.chars() {
            secret.push(ch);
        }
        secret
    }

    #[test]
    fn secret_edits_like_a_text_field() {
        let mut value = secret("héllo");
        assert_eq!(value.chars(), 5);
        value.pop();
        value.pop();
        assert_eq!(value.as_str(), "hél");
        let taken = value.take();
        assert!(value.is_empty());
        assert_eq!(taken.as_str(), "hél");
    }

    #[test]
    fn secret_never_reallocates() {
        let mut value = Secret::new();
        let capacity = value.0.capacity();
        for _ in 0..10_000 {
            value.push('x');
        }
        assert_eq!(value.0.capacity(), capacity, "typing must not move the buffer");
    }

    #[test]
    fn missing_services_are_never_chosen() {
        let dir = std::env::temp_dir().join(format!("stratum-pam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(resolve_service("nope", Some(&dir)), None);
        std::fs::write(dir.join("login"), "").unwrap();
        assert_eq!(resolve_service("nope", Some(&dir)).as_deref(), Some("login"));
        std::fs::write(dir.join("custom"), "").unwrap();
        assert_eq!(resolve_service("custom", Some(&dir)).as_deref(), Some("custom"));
        // Paths are not service names.
        assert_eq!(resolve_service("../custom", Some(&dir)).as_deref(), Some("login"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Run real PAM transactions against services whose outcome is fixed:
    /// `pam_exec` runs a script that accepts exactly one password.
    #[test]
    fn authenticates_through_real_pam() {
        let Some(pam_exec) = find_module("pam_exec.so") else {
            eprintln!("pam_exec.so not found; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("stratum-pam-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("check.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nread -r password\n[ \"$password\" = \"correct horse\" ]\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let permit = pam_exec.with_file_name("pam_permit.so");
        std::fs::write(
            dir.join("stratum-test"),
            format!(
                "auth required {} expose_authtok quiet {}\naccount required {}\n",
                pam_exec.display(),
                script.display(),
                permit.display()
            ),
        )
        .unwrap();

        let user = current_user().expect("a user name");
        assert_eq!(
            authenticate("stratum-test", &user, secret("correct horse"), Some(&dir)),
            Ok(())
        );
        assert!(authenticate("stratum-test", &user, secret("wrong"), Some(&dir)).is_err());
        assert!(authenticate("stratum-test", &user, secret(""), Some(&dir)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn service_check_loads_modules_and_follows_includes() {
        let Some(permit) = find_module("pam_permit.so") else {
            eprintln!("pam_permit.so not found; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("stratum-pam-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let permit = permit.display();
        std::fs::write(
            dir.join("common"),
            format!("# comment\nauth [success=1 default=ignore] {permit}\n-auth optional /nope.so\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("good"),
            format!("auth include common\naccount required {permit}\nsession required /nope.so\n"),
        )
        .unwrap();
        assert_eq!(check_service("good", Some(&dir)), Ok(()));

        // A module this process cannot load fails the check, through an include.
        std::fs::write(dir.join("broken"), "auth required /nonexistent/pam_unix.so\n").unwrap();
        std::fs::write(dir.join("outer"), "auth substack broken\n").unwrap();
        let err = check_service("outer", Some(&dir)).unwrap_err();
        assert!(err.contains("/nonexistent/pam_unix.so"), "{err}");

        // Include cycles end instead of recursing forever.
        std::fs::write(dir.join("loop"), "auth include loop\n").unwrap();
        assert!(check_service("loop", Some(&dir)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn find_module(name: &str) -> Option<PathBuf> {
        // The module directory sits beside the libpam the binary linked.
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        maps.lines()
            .filter_map(|line| line.split_whitespace().last())
            .filter(|path| path.contains("libpam.so"))
            .map(|path| Path::new(path).with_file_name("security").join(name))
            .find(|path| path.is_file())
    }
}
