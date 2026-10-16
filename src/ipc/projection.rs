//! Process-local immutable derived documents. A cache entry is not an action
//! approval. Consumers supply their own canonical key and live-source checks.
use crate::Result;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(crate) enum View<T> {
    Preparing,
    Unavailable,
    Ready(Arc<T>),
}
impl<T> View<T> {
    pub fn state(&self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Unavailable => "unavailable",
            Self::Ready(_) => "ready",
        }
    }
    pub fn document(&self) -> Option<&T> {
        match self {
            Self::Ready(document) => Some(document),
            _ => None,
        }
    }
}
struct Entry<T> {
    key: String,
    generation: String,
    result: Option<Option<Arc<T>>>,
}
pub(crate) struct Cache<T>(Mutex<BTreeMap<String, Entry<T>>>);
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }
}
impl<T: Send + Sync + 'static> Cache<T> {
    pub fn prepare(
        self: &Arc<Self>,
        run_id: String,
        key: String,
        build: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> View<T> {
        let generation = crate::id();
        {
            let mut entries = self.0.lock().expect("projection mutex");
            if let Some(entry) = entries.get(&run_id).filter(|e| e.key == key) {
                return match &entry.result {
                    None => View::Preparing,
                    Some(None) => View::Unavailable,
                    Some(Some(document)) => View::Ready(Arc::clone(document)),
                };
            }
            entries.insert(
                run_id.clone(),
                Entry {
                    key: key.clone(),
                    generation: generation.clone(),
                    result: None,
                },
            );
        }
        let cache = Arc::clone(self);
        std::thread::spawn(move || {
            let result = build().ok().map(Arc::new);
            let mut entries = cache.0.lock().expect("projection mutex");
            if let Some(entry) = entries.get_mut(&run_id)
                && entry.key == key
                && entry.generation == generation
            {
                entry.result = Some(result);
            }
        });
        View::Preparing
    }
    pub fn clear(&self, run_id: &str) {
        self.0.lock().expect("projection mutex").remove(run_id);
    }
    pub fn remove(&self, run_id: &str, key: &str) {
        let mut entries = self.0.lock().expect("projection mutex");
        if entries.get(run_id).is_some_and(|e| e.key == key) {
            entries.remove(run_id);
        }
    }
    pub fn peek(&self, run_id: &str, key: &str) -> Option<Arc<T>> {
        self.0
            .lock()
            .expect("projection mutex")
            .get(run_id)
            .filter(|e| e.key == key)
            .and_then(|e| e.result.as_ref())
            .and_then(|r| r.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    struct Value {
        value: &'static str,
        dropped: Option<mpsc::Sender<()>>,
    }
    impl Drop for Value {
        fn drop(&mut self) {
            if let Some(sender) = &self.dropped {
                let _ = sender.send(());
            }
        }
    }
    #[test]
    fn abandoned_worker_cannot_publish_into_a_new_generation_of_the_same_key() {
        let cache = Arc::new(Cache::default());
        let (started, start) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (dropped, drop_observed) = mpsc::channel();
        assert!(matches!(
            cache.prepare("run".into(), "same-key".into(), move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                Ok(Value {
                    value: "abandoned",
                    dropped: Some(dropped),
                })
            }),
            View::Preparing
        ));
        start.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            cache.prepare("run".into(), "same-key".into(), || panic!(
                "duplicate background job"
            )),
            View::Preparing
        ));
        cache.remove("run", "same-key");
        assert!(matches!(
            cache.prepare("run".into(), "same-key".into(), || Ok(Value {
                value: "current",
                dropped: None
            })),
            View::Preparing
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        let current = loop {
            if let Some(value) = cache.peek("run", "same-key") {
                break value;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        release.send(()).unwrap();
        drop_observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let after = cache.peek("run", "same-key").unwrap();
        assert_eq!(after.value, "current");
        assert!(Arc::ptr_eq(&current, &after));
        cache.remove("run", "older-key");
        assert!(cache.peek("run", "same-key").is_some());
    }
}
