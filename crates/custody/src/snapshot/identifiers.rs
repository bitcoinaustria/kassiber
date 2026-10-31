//! Book-local identifier sharing. No identifier survives here after parsing;
//! rows own the Arc handles, and independent or nested parses have separate pools.
use std::{cell::RefCell, collections::HashSet, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InternedId(Arc<str>);
impl InternedId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// Share with the active book parse, or own a standalone identifier when
    /// called outside that scope. Stage models can retain the returned handle.
    pub fn new(value: &str) -> Self {
        POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let Some(pool) = pool.as_mut() else {
                return Self(Arc::from(value));
            };
            if let Some(id) = pool.get(value) {
                return Self(id.clone());
            }
            let id: Arc<str> = Arc::from(value);
            pool.insert(id.clone());
            Self(id)
        })
    }
}
impl AsRef<str> for InternedId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
thread_local! { static POOL: RefCell<Option<HashSet<Arc<str>>>> = const { RefCell::new(None) }; }
pub(super) struct Scope(Option<HashSet<Arc<str>>>);
impl Scope {
    pub(super) fn enter() -> Self {
        Self(POOL.with(|pool| pool.replace(Some(HashSet::new()))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        POOL.with(|pool| pool.replace(self.0.take()));
    }
}
