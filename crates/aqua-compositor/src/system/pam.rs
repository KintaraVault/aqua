//! Minimal PAM binding used by the lock screen (runs on a worker thread).
use std::ffi::{c_char, c_int, c_void, CStr, CString};

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_SUCCESS: c_int = 0;
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
#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, pamh: *mut *mut c_void) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut c_void, errnum: c_int) -> *const c_char;
}

extern "C" fn conv(n: c_int, msgs: *mut *const PamMessage, resp: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    unsafe {
        if n <= 0 || n > 32 {
            return PAM_CONV_ERR;
        }
        let pw = &*(data as *const CString);
        let out = libc::calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if out.is_null() {
            return PAM_CONV_ERR;
        }
        for i in 0..n as isize {
            let m = &**msgs.offset(i);
            let r = &mut *out.offset(i);
            r.resp_retcode = 0;
            r.resp = match m.msg_style {
                PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => libc::strdup(pw.as_ptr()),
                _ => std::ptr::null_mut(),
            };
        }
        *resp = out;
        PAM_SUCCESS
    }
}

/// Which PAM service to use: /etc/pam.d/aqua if installed, else common fallbacks.
pub fn service() -> &'static str {
    for s in ["aqua", "swaylock", "login", "system-auth"] {
        if std::path::Path::new("/etc/pam.d").join(s).exists() {
            return s;
        }
    }
    "login"
}

/// Check `password` for `user`. Blocking.
pub fn authenticate(user: &str, password: &str) -> Result<(), String> {
    let service = CString::new(service()).unwrap();
    let user = CString::new(user).map_err(|_| "bad user")?;
    let pw = Box::new(CString::new(password).map_err(|_| "bad password")?);
    let c = PamConv { conv, appdata_ptr: &*pw as *const CString as *mut c_void };
    let mut h: *mut c_void = std::ptr::null_mut();
    unsafe {
        let r = pam_start(service.as_ptr(), user.as_ptr(), &c, &mut h);
        if r != PAM_SUCCESS {
            return Err(format!("pam_start failed ({r})"));
        }
        let mut r = pam_authenticate(h, 0);
        if r == PAM_SUCCESS {
            r = pam_acct_mgmt(h, 0);
            if r != PAM_SUCCESS {
                tracing::warn!("pam_acct_mgmt: {}", CStr::from_ptr(pam_strerror(h, r)).to_string_lossy());
                r = PAM_SUCCESS;
            }
            pam_setcred(h, PAM_REFRESH_CRED);
        }
        let msg = if r != PAM_SUCCESS {
            Some(CStr::from_ptr(pam_strerror(h, r)).to_string_lossy().to_string())
        } else {
            None
        };
        pam_end(h, r);
        match msg {
            None => Ok(()),
            Some(m) => Err(m),
        }
    }
}

/// Login name and display name (GECOS) of the session user.
pub fn current_user() -> (String, String) {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            let u = std::env::var("USER").unwrap_or_else(|_| "user".into());
            return (u.clone(), u);
        }
        let name = CStr::from_ptr((*pw).pw_name).to_string_lossy().to_string();
        let gecos = if (*pw).pw_gecos.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*pw).pw_gecos).to_string_lossy().split(',').next().unwrap_or("").to_string()
        };
        let real = if gecos.trim().is_empty() { name.clone() } else { gecos };
        (name, real)
    }
}
