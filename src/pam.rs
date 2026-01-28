// SPDX-License-Identifier: MIT
//
// Author: Johannes Leupolz <dev@leupolz.eu>
// Original Author: Florian Wilkens <gh@1wilkens.org>
//
// this is basically a clone of pam/client.rs of https://crates.io/crates/pam
// - with added get_env and set_env functions,
// - stripped from everything that I don't need for fallbackdm.

use std::ffi::{CStr, CString};

use libc::{c_int, c_void, calloc, free, size_t, strdup};

use pam::ffi::pam_conv;
use pam::*;
use std::mem;

/// Main struct to authenticate a user
///
/// You need to create an instance of it to start an authentication process. If you
/// want a simple password-based authentication, you can use `Client::with_password`,
/// and to the following flow:
///
/// ```no_run
/// use pam::AdvClient;
///
/// let mut client = AdvClient::with_password("system-auth")
///         .expect("Failed to init PAM client.");
/// // Preset the login & password we will use for authentication
/// client.conversation_mut().set_credentials("login", "password");
/// // Actually try to authenticate:
/// client.authenticate().expect("Authentication failed!");
/// // Now that we are authenticated, it's possible to open a sesssion:
/// client.open_session().expect("Failed to open a session!");
/// ```
///
/// If you wish to customise the PAM conversation function, you should rather create your
/// client with `Client::with_handler`, providing a struct implementing the
/// `conv::Conversation` trait. You can then mutably access your conversation handler using the
/// `Client::handler_mut` method.
///
/// By default, the `Client` will close any opened session when dropped. If you don't
/// want this, you can change its `close_on_drop` field to `False`.
pub struct PasswordlessClient<'a> {
    /// Flag indicating whether the Client should close the session on drop
    pub close_on_drop: bool,
    conversation: Box<SimpleConv>,
    handle: &'a mut PamHandle,
    is_authenticated: bool,
    has_open_session: bool,
    last_code: PamReturnCode,
}

impl<'a> PasswordlessClient<'a> {
    /// Create a new `Client` with the given service name
    pub fn new_client(service: &str) -> PamResult<PasswordlessClient<'a>> {
        let mut conversation = Box::new(SimpleConv::new());
        let conv = into_pam_conv(&mut *conversation);

        let handle = start(service, None, &conv)?;
        Ok(PasswordlessClient {
            close_on_drop: true,
            conversation,
            handle,
            is_authenticated: false,
            has_open_session: false,
            last_code: PamReturnCode::Success,
        })
    }

    /// Perform authentication with the provided credentials
    pub fn authenticate(&mut self) -> PamResult<()> {
        self.last_code = authenticate(self.handle, PamFlag::None);
        if self.last_code != PamReturnCode::Success {
            return Err(From::from(self.last_code));
        }

        self.is_authenticated = true;
        Ok(())
    }

    /// Open a session for a previously authenticated user and
    /// initialize the environment appropriately (in PAM and regular enviroment variables).
    pub fn open_session(&mut self) -> PamResult<()> {
        if !self.is_authenticated {
            return Err(PamReturnCode::Perm_Denied.into());
        }

        self.last_code = open_session(self.handle, false);
        if self.last_code != PamReturnCode::Success {
            return Err(From::from(self.last_code));
        }

        self.has_open_session = true;
        Ok(())
    }

    // Utility function to set an environment variable in PAM and the process
    pub fn get_env(&mut self, key: &str) -> PamResult<Option<String>> {
        getenv(self.handle, key).map(|opt| opt.map(|s| s.to_owned()))
    }

    pub fn set_env(&mut self, key: &str, value: &str) -> PamResult<()> {
        let env = format!("{}={}", key, value);
        putenv(self.handle, &env)
    }
}

impl<'a> Drop for PasswordlessClient<'a> {
    fn drop(&mut self) {
        let mut result = PamReturnCode::Success;
        if self.has_open_session && self.close_on_drop {
            result = close_session(self.handle, false);
        }
        end(self.handle, result);
    }
}

/// A minimalistic conversation handler
pub struct SimpleConv {}

impl SimpleConv {
    /// Create a new `PasswordConv` handler
    pub fn new() -> SimpleConv {
        SimpleConv {}
    }
}

impl Conversation for SimpleConv {
    fn prompt_echo(&mut self, _msg: &CStr) -> Result<CString, ()> {
        CString::new("root".to_string()).map_err(|_| ())
    }
    fn prompt_blind(&mut self, _msg: &CStr) -> Result<CString, ()> {
        CString::new("no password".to_string()).map_err(|_| ())
    }
    fn info(&mut self, _msg: &CStr) {}
    fn error(&mut self, msg: &CStr) {
        eprintln!("[PAM ERROR] {}", msg.to_string_lossy());
    }
}

fn into_pam_conv(conv: &mut SimpleConv) -> pam_conv {
    pam_conv {
        conv: Some(converse::<SimpleConv>),
        appdata_ptr: conv as *mut SimpleConv as *mut c_void,
    }
}

// FIXME: verify this
pub(crate) unsafe extern "C" fn converse<C: Conversation>(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    out_resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    // allocate space for responses
    let resp =
        calloc(num_msg as usize, mem::size_of::<PamResponse>() as size_t) as *mut PamResponse;
    if resp.is_null() {
        return PamReturnCode::Buf_Err as c_int;
    }

    let handler = &mut *(appdata_ptr as *mut C);

    let mut result: PamReturnCode = PamReturnCode::Success;
    for i in 0..num_msg as isize {
        // get indexed values
        // FIXME: check this
        let m: &mut PamMessage = &mut *(*(msg.offset(i)) as *mut PamMessage);
        let r: &mut PamResponse = &mut *(resp.offset(i));

        let msg = CStr::from_ptr(m.msg);
        // match on msg_style
        match PamMessageStyle::from(m.msg_style) {
            PamMessageStyle::Prompt_Echo_On => {
                if let Ok(handler_response) = handler.prompt_echo(msg) {
                    r.resp = strdup(handler_response.as_ptr());
                } else {
                    result = PamReturnCode::Conv_Err;
                }
            }
            PamMessageStyle::Prompt_Echo_Off => {
                if let Ok(handler_response) = handler.prompt_blind(msg) {
                    r.resp = strdup(handler_response.as_ptr());
                } else {
                    result = PamReturnCode::Conv_Err;
                }
            }
            PamMessageStyle::Text_Info => {
                handler.info(msg);
            }
            PamMessageStyle::Error_Msg => {
                handler.error(msg);
                result = PamReturnCode::Conv_Err;
            }
        }
        if result != PamReturnCode::Success {
            break;
        }
    }

    // free allocated memory if an error occured
    if result != PamReturnCode::Success {
        free(resp as *mut c_void);
    } else {
        *out_resp = resp;
    }

    result as c_int
}
