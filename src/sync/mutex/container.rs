use downcast_rs::DowncastSync;

use crate::core::fmt;
use crate::alloc::sync::Arc;

pub trait MutexDataSync: DowncastSync {

}

impl_downcast!(sync MutexDataSync);

pub struct MutexDataContainerSync(Option<Arc<dyn MutexDataSync>>);

impl MutexDataContainerSync {
    pub const fn empty() -> Self {
        MutexDataContainerSync(None)
    }

    pub fn clear(&mut self) {
        self.0.take();
    }

    pub fn take(&mut self) -> Option<Arc<dyn MutexDataSync>> {
        self.0.take()
    }

    pub fn replace(&mut self, item: Arc<dyn MutexDataSync>) {
        self.0.replace(item);
    }

    pub fn has_data(&self) -> bool {
        self.0.is_some()
    }

    pub fn as_dyn_arc(&self) -> Option<&Arc<dyn MutexDataSync>> {
        self.0.as_ref()
    }

    pub fn as_dyn(&self) -> Option<&dyn MutexDataSync> {
        self.as_dyn_arc().map(|x| x.as_ref())
    }

    pub fn as_ref<C>(&self) -> Option<&C> where C: MutexDataSync {
        match self.as_dyn_arc() {
            Some(e) => e.downcast_ref::<C>(),
            None => None,
        }
    }

    pub fn as_arc<C>(&self) -> Option<Arc<C>> where C: MutexDataSync {
        match self.as_dyn_arc() {
            Some(e) => e.clone().downcast_arc::<C>().ok(),
            None => None,
        }
    }
}

impl fmt::Debug for MutexDataContainerSync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutexDataContainerSync").finish()
    }
}
