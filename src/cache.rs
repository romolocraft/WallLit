use std::collections::HashMap;
use std::hash::Hash;

pub struct BudgetCache<K, V> {
    entries: HashMap<K, (V, usize, u64)>,
    bytes: usize,
    budget: usize,
    limit: usize,
    clock: u64,
}
impl<K: Eq + Hash + Clone, V> BudgetCache<K, V> {
    pub fn new(budget: usize, limit: usize) -> Self {
        Self { entries: HashMap::new(), bytes: 0, budget, limit, clock: 0 }
    }
    pub fn clear(&mut self) { self.entries.clear(); self.bytes = 0; }
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        self.entries.get_mut(key).map(|(value, _, stamp)| { *stamp = self.clock; &*value })
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if bytes > self.budget || self.limit == 0 { return; }
        if let Some((_, weight, _)) = self.entries.remove(&key) { self.bytes -= weight; }
        while !self.entries.is_empty() && (self.bytes + bytes > self.budget || self.entries.len() >= self.limit) {
            let oldest = self.entries.iter().min_by_key(|(_, (_, _, stamp))| stamp).map(|(key, _)| key.clone()).unwrap();
            self.bytes -= self.entries.remove(&oldest).unwrap().1;
        }
        self.clock += 1;
        self.bytes += bytes;
        self.entries.insert(key, (value, bytes, self.clock));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evicts_least_used_and_rejects_oversized_values() {
        let mut cache = BudgetCache::new(10, 3);
        cache.insert(1, 'a', 4); cache.insert(2, 'b', 4);
        assert_eq!(cache.get(&1), Some(&'a'));
        cache.insert(3, 'c', 4);
        assert!(cache.get(&2).is_none());
        assert_eq!(cache.get(&1), Some(&'a'));
        cache.insert(4, 'd', 11);
        assert!(cache.get(&4).is_none());
    }
}
