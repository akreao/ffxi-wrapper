//! A small in-memory cache for the public JSON, so visitors can't load the database:
//! an answer lives for 30 seconds, and any edit in the admin panel drops all of them.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

pub const TTL: i64 = 30;

/// Keys are `home`, `news:<page>`, `post:<id>`, `content:<key>`; callers only store answers
/// that exist, so the number of entries is bounded by what has been published.
#[derive(Default)]
pub struct PublicCache {
    entries: Mutex<HashMap<String, (i64, Value)>>,
}

impl PublicCache {
    pub fn get(&self, key: &str, now: i64) -> Option<Value> {
        let map = self.entries.lock().unwrap();
        map.get(key).filter(|(at, _)| now - at < TTL).map(|(_, v)| v.clone())
    }

    pub fn put(&self, key: &str, value: &Value, now: i64) {
        let mut map = self.entries.lock().unwrap();
        // Old entries would otherwise stay until the next edit.
        map.retain(|_, (at, _)| now - *at < TTL);
        map.insert(key.to_string(), (now, value.clone()));
    }

    /// Called after every edit of news or site content.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
