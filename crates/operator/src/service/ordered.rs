//! An insertion-ordered map with `move_to_end`, for the `OrderedDict`s the
//! service keeps (operations, tombstones, backoffs).

use std::collections::{BTreeMap, HashMap};

pub(crate) struct Ordered<V> {
    next: u64,
    order: BTreeMap<u64, String>,
    items: HashMap<String, (u64, V)>,
}

impl<V> Default for Ordered<V> {
    fn default() -> Self {
        Ordered {
            next: 0,
            order: BTreeMap::new(),
            items: HashMap::new(),
        }
    }
}

impl<V> Ordered<V> {
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        self.items.get(key).map(|(_, value)| value)
    }

    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.items.get_mut(key).map(|(_, value)| value)
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.items.contains_key(key)
    }

    /// `map[key] = value`: a new key goes last; an existing key keeps its
    /// place.
    pub(crate) fn insert(&mut self, key: String, value: V) {
        if let Some(slot) = self.items.get_mut(&key) {
            slot.1 = value;
            return;
        }
        let position = self.next;
        self.next += 1;
        self.order.insert(position, key.clone());
        self.items.insert(key, (position, value));
    }

    pub(crate) fn move_to_end(&mut self, key: &str) {
        let Some(slot) = self.items.get_mut(key) else {
            return;
        };
        self.order.remove(&slot.0);
        slot.0 = self.next;
        self.order.insert(self.next, key.to_owned());
        self.next += 1;
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<V> {
        let (position, value) = self.items.remove(key)?;
        self.order.remove(&position);
        Some(value)
    }

    /// `popitem(last=False)`.
    pub(crate) fn pop_first(&mut self) -> Option<(String, V)> {
        let (_, key) = self.order.pop_first()?;
        let (_, value) = self.items.remove(&key)?;
        Some((key, value))
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> {
        self.order.values()
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.order.values().map(|key| &self.items[key].1)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.order.values().map(|key| (key, &self.items[key].1))
    }

    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.contains(key)
    }
}

impl<V> std::ops::Index<&str> for Ordered<V> {
    type Output = V;

    fn index(&self, key: &str) -> &V {
        self.get(key).expect("key present")
    }
}

impl<V> std::ops::Index<&String> for Ordered<V> {
    type Output = V;

    fn index(&self, key: &String) -> &V {
        self.get(key).expect("key present")
    }
}
