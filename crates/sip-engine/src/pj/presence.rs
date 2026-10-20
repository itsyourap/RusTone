//! Presence subscriptions.
//! Buddies are shared by numbers that format to the same URI.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use pjsua_sys::*;

use super::util::{pj_default, pj_to_string};
use crate::presence::{Activity, BuddyStatus, Presence, classify};

/// The numbers each buddy stands for, read by the presence callback.
static NUMBERS: LazyLock<Mutex<HashMap<pjsua_buddy_id, Vec<String>>>> = LazyLock::new(Mutex::default);

fn numbers() -> std::sync::MutexGuard<'static, HashMap<pjsua_buddy_id, Vec<String>>> {
    NUMBERS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The session's subscriptions: buddy URI → buddy id.
#[derive(Default)]
pub struct Buddies {
    by_uri: HashMap<String, pjsua_buddy_id>,
}

impl Buddies {
    /// Subscribes `wanted` (number → URI) from account `acc` and drops
    /// buddies no longer wanted.
    pub fn sync(&mut self, wanted: &[(String, String)], acc: pjsua_acc_id, arena: &mut super::util::Arena) {
        let mut groups: HashMap<&str, Vec<String>> = HashMap::new();
        for (number, uri) in wanted {
            groups.entry(uri.as_str()).or_default().push(number.clone());
        }
        self.by_uri.retain(|uri, id| {
            let keep = groups.contains_key(uri.as_str());
            if !keep {
                // SAFETY: a buddy of this session.
                unsafe { pjsua_buddy_del(*id) };
                numbers().remove(id);
            }
            keep
        });
        for (uri, group) in groups {
            if let Some(id) = self.by_uri.get(uri) {
                numbers().insert(*id, group);
                // Replay the known state for numbers that just joined.
                emit_state(*id);
                continue;
            }
            pj_default!(cfg: pjsua_buddy_config = pjsua_buddy_config_default);
            cfg.uri = arena.pj(uri);
            cfg.subscribe = PJ_TRUE;
            // Pinned to the main account: left to pjsua, the local account
            // can win and its transport can't reach the server.
            cfg.acc_id = acc;
            let mut id = PJSUA_INVALID_ID;
            // Registered before adding: the first NOTIFY can come at once.
            // SAFETY: `cfg` initialised in place; the URI lives in the arena.
            let status = unsafe { pjsua_buddy_add(&cfg, &mut id) };
            if status == PJ_SUCCESS {
                numbers().insert(id, group);
                self.by_uri.insert(uri.to_owned(), id);
                emit_state(id);
            } else {
                log::warn!("cannot subscribe to the presence of {uri}: {}", super::util::error_text(status));
            }
        }
    }

    /// Drops every subscription (account change).
    pub fn clear(&mut self) {
        for (_, id) in self.by_uri.drain() {
            // SAFETY: a buddy of this session.
            unsafe { pjsua_buddy_del(id) };
            numbers().remove(&id);
        }
    }
}

/// pjsua's on_buddy_state.
pub unsafe extern "C" fn on_buddy_state(buddy_id: pjsua_buddy_id) {
    super::callbacks::guard(|| emit_state(buddy_id));
}

/// Reports the buddy's state for each number that uses it.
fn emit_state(buddy_id: pjsua_buddy_id) {
    let Some(group) = numbers().get(&buddy_id).cloned() else { return };
    let mut info = std::mem::MaybeUninit::<pjsua_buddy_info>::zeroed();
    // SAFETY: filled on success and read in place (its strings point into it).
    let (state, note) = unsafe {
        if pjsua_buddy_get_info(buddy_id, info.as_mut_ptr()) != PJ_SUCCESS {
            return;
        }
        let info = info.assume_init_ref();
        let status = match info.status {
            PJSUA_BUDDY_STATUS_ONLINE => BuddyStatus::Online,
            PJSUA_BUDDY_STATUS_OFFLINE => BuddyStatus::Offline,
            _ => BuddyStatus::Unknown,
        };
        let activity = match info.rpid.activity {
            PJRPID_ACTIVITY_AWAY => Activity::Away,
            PJRPID_ACTIVITY_BUSY => Activity::Busy,
            _ => Activity::Other,
        };
        let note = pj_to_string(&info.status_text);
        (classify(status, activity, &note), note)
    };
    for number in group {
        super::callbacks::emit(crate::Event::Presence(Presence { number, state, note: note.clone() }));
    }
}
