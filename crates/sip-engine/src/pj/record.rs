//! Call recording to WAV: the
//! call's audio and our microphone into one file, shared by every member of
//! a conference. Functions here run on the engine thread or on pjsua's
//! callback threads; state is behind a mutex.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use pjsua_sys::*;

use super::calls;
use super::util::{error_text, pj_to_string};
use crate::call::CallId;

/// Where recordings go and whether calls are recorded when answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingOptions {
    /// The folder; created when needed.
    pub dir: PathBuf,
    /// Record every call once it's answered.
    pub auto: bool,
    /// The account's name for file names (its label).
    pub label: String,
}

struct Recorder {
    id: pjsua_recorder_id,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    options: RecordingOptions,
    /// Each recording call's recorder; conference members share one.
    recorders: HashMap<CallId, std::sync::Arc<Recorder>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_options(options: RecordingOptions) {
    lock().options = options;
}

pub fn auto_record() -> bool {
    lock().options.auto
}

pub fn is_recording(call: CallId) -> bool {
    lock().recorders.contains_key(&call)
}

/// Starts recording `call`, joining a conference member's recording when
/// there is one. Returns the file.
pub fn start(call: CallId) -> Result<PathBuf, String> {
    calls::call_slot(call).ok_or("the call has no audio yet")?;
    let members = calls::conference_members(call);
    let mut state = lock();
    if let Some(r) = state.recorders.get(&call) {
        return Ok(r.path.clone());
    }
    let shared = members.iter().find_map(|m| state.recorders.get(m).cloned());
    let recorder = match shared {
        Some(r) => r,
        None => {
            let path = file_for(call, &state.options).ok_or("the call has ended")?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
            }
            let file = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "bad file name")?;
            let name = pj_str_t { ptr: file.as_ptr().cast_mut(), slen: file.as_bytes().len() as pj_ssize_t };
            let mut id = PJSUA_INVALID_ID;
            // SAFETY: `name` lives until the call returns; pjsua copies it.
            let status = unsafe { pjsua_recorder_create(&name, 0, std::ptr::null_mut(), -1, 0, &mut id) };
            if status != PJ_SUCCESS {
                return Err(format!("Can't record to {}: {}", path.display(), error_text(status)));
            }
            // SAFETY: a recorder this session just made; slot 0 is the microphone.
            unsafe { pjsua_conf_connect(0, pjsua_recorder_get_conf_port(id)) };
            std::sync::Arc::new(Recorder { id, path })
        }
    };
    // SAFETY: valid slots of this pjsua instance.
    let port = unsafe { pjsua_recorder_get_conf_port(recorder.id) };
    for member in std::iter::once(call).chain(members) {
        if let Some(member_slot) = calls::call_slot(member) {
            // SAFETY: as above.
            unsafe { pjsua_conf_connect(member_slot, port) };
            state.recorders.entry(member).or_insert_with(|| recorder.clone());
        }
    }
    Ok(recorder.path.clone())
}

/// Stops recording `call`. The file is closed once no call uses it;
/// returns it then.
pub fn stop(call: CallId) -> Option<PathBuf> {
    let mut state = lock();
    let recorder = state.recorders.remove(&call)?;
    // SAFETY: valid ids of this pjsua instance; a gone call's slot is ignored.
    unsafe {
        if let Some(slot) = calls::call_slot(call) {
            pjsua_conf_disconnect(slot, pjsua_recorder_get_conf_port(recorder.id));
        }
    }
    if state.recorders.values().any(|r| r.id == recorder.id) {
        return None;
    }
    // SAFETY: nothing else uses it any more.
    unsafe { pjsua_recorder_destroy(recorder.id) };
    Some(recorder.path.clone())
}

/// Stops every recording (session end).
pub fn stop_all() {
    let calls: Vec<CallId> = lock().recorders.keys().copied().collect();
    for call in calls {
        stop(call);
    }
}

/// Joins a call that entered a conference to the conference's recording,
/// or brings the conference into the
/// call's recording.
pub fn follow_conference(call: CallId) {
    let members = calls::conference_members(call);
    let recording = {
        let state = lock();
        std::iter::once(call).chain(members.iter().copied()).any(|c| state.recorders.contains_key(&c))
    };
    if recording {
        let first = std::iter::once(call).chain(members).find(|c| lock().recorders.contains_key(c));
        if let Some(first) = first
            && let Err(e) = start(first)
        {
            log::warn!("{e}");
        }
    }
}

/// File name: `20261020-142233-100-incoming-Home.wav`, with
/// characters a file name can't hold replaced by `_`.
fn file_for(call: CallId, options: &RecordingOptions) -> Option<PathBuf> {
    calls::with_call_info(call, |ci| {
        let remote = pj_to_string(&ci.remote_info);
        let local = pj_to_string(&ci.local_info);
        let user = |uri: &str| {
            let user_host = calls::uri_user_host(uri);
            user_host.split('@').next().unwrap_or(&user_host).to_owned()
        };
        let direction = if ci.role == PJSIP_ROLE_UAC { "outgoing" } else { "incoming" };
        let account = if options.label.is_empty() { user(&local) } else { options.label.clone() };
        (user(&remote), direction, account)
    })
    .map(|(remote, direction, account)| file_in(&options.dir, &local_stamp(), &remote, direction, &account))
}

fn file_in(dir: &Path, stamp: &str, remote: &str, direction: &str, account: &str) -> PathBuf {
    let name: String = format!("{stamp}-{remote}-{direction}-{account}")
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | '?' | '%' | '*' | ':' | '|' | '"' | '<' | '>' | '.' | ' ') { '_' } else { c })
        .collect();
    dir.join(format!("{name}.wav"))
}

/// Local time as `YYYYMMDD-HHMMSS`.
fn local_stamp() -> String {
    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_carry_time_number_and_account() {
        assert_eq!(local_stamp().len(), "20261020-142233".len());
        let path = file_in(Path::new("C:/rec"), "20261020-142233", "+91 345.7", "incoming", "Home: Office");
        assert_eq!(path, Path::new("C:/rec").join("20261020-142233-+91_345_7-incoming-Home__Office.wav"));
    }
}
