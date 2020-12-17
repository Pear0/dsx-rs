use core::time::Duration;
use crate::sync::{AtomicBool, Ordering};

pub trait SystemHooks {

    fn current_time(&self) -> Duration;

}

static mut HOOKS: Option<&'static dyn SystemHooks> = None;
static STATE: AtomicBool = AtomicBool::new(false);

pub fn maybe_system_hooks() -> Option<&'static dyn SystemHooks> {
    if !STATE.load(Ordering::Acquire) {
        return None;
    }

    Some(unsafe { HOOKS }.expect("STATE == true, but HOOKS is None"))
}

pub fn system_hooks() -> &'static dyn SystemHooks {
    maybe_system_hooks().expect("[dsx] system_hooks not connected")
}

pub unsafe fn set_system_hooks(hooks: &'static dyn SystemHooks) {
    HOOKS = Some(hooks);
    STATE.store(true, Ordering::Release);
}