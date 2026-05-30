use std::iter;

// ---------------------------------------------------------------------------
// ArenaId
// ---------------------------------------------------------------------------

/// A generation-based ID for arena entries.
///
/// `index` is the slot position; `generation` is incremented each time the
/// slot is freed so that stale IDs can never accidentally reach new data.
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct ArenaId {
    pub index: usize,
    pub generation: u64,
}

impl ArenaId {
    pub const fn new(index: usize, generation: u64) -> Self {
        Self { index, generation }
    }
}

// ---------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------

const DEFAULT_CAPACITY: usize = 1024;

/// A pre-allocated, generation-based arena allocator.
///
/// `Arena<T>` stores values in a contiguous `Vec<T>` and tracks liveness with
/// parallel metadata vectors.  Allocation reuses freed slots via a free list
/// and only grows when new capacity is needed past the pre-allocated reserve.
/// De-allocation increments the entry's generation so that stale IDs become
/// invalid.
pub struct Arena<T: Default + Clone> {
    storage: Vec<T>,
    free_list: Vec<usize>,
    generation: Vec<u64>,
    alive: Vec<bool>,
    capacity: usize,
    next_alloc: usize,
}

impl<T: Default + Clone> Arena<T> {
    /// Create a new arena pre-allocated for `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let storage = iter::repeat_with(T::default).take(cap).collect();
        Self {
            storage,
            free_list: Vec::new(),
            generation: vec![0u64; cap],
            alive: vec![false; cap],
            capacity: cap,
            next_alloc: 0,
        }
    }

    /// Allocate a slot for `value`, returning an `ArenaId` or `None` if the
    /// arena is full (no free slots and `capacity` exhausted).
    ///
    /// This method prefers the free list first and falls back to the next
    /// unused sequential slot.
    pub fn alloc(&mut self, value: T) -> Option<ArenaId> {
        // Prefer free list
        if let Some(index) = self.free_list.pop() {
            debug_assert!(!self.alive[index]);
            self.storage[index] = value;
            self.alive[index] = true;
            // generation stays the same (already incremented on dealloc)
            return Some(ArenaId::new(index, self.generation[index]));
        }

        // Use next available slot
        if self.next_alloc < self.capacity {
            let index = self.next_alloc;
            self.next_alloc += 1;
            self.storage[index] = value;
            self.alive[index] = true;
            // generation already 0 from init
            Some(ArenaId::new(index, 0))
        } else {
            None
        }
    }

    /// Deallocate the entry at `id`.
    ///
    /// Returns `false` if the ID is stale (generation mismatch) or already
    /// freed.
    pub fn dealloc(&mut self, id: ArenaId) -> bool {
        if id.index >= self.storage.len() {
            return false;
        }
        if self.generation[id.index] != id.generation {
            return false;
        }
        if !self.alive[id.index] {
            return false;
        }

        self.alive[id.index] = false;
        self.generation[id.index] = self.generation[id.index].wrapping_add(1);
        self.free_list.push(id.index);

        // Drop the value (write default so old data isn't accidentally referenced)
        self.storage[id.index] = T::default();

        true
    }

    /// Borrow the value at `id`, checking generation.  Returns `None` for
    /// stale or freed slots.
    pub fn get(&self, id: ArenaId) -> Option<&T> {
        if id.index >= self.storage.len() {
            return None;
        }
        if !self.alive[id.index] {
            return None;
        }
        if self.generation[id.index] != id.generation {
            return None;
        }
        Some(&self.storage[id.index])
    }

    /// Mutably borrow the value at `id`, checking generation.
    pub fn get_mut(&mut self, id: ArenaId) -> Option<&mut T> {
        if id.index >= self.storage.len() {
            return None;
        }
        if !self.alive[id.index] {
            return None;
        }
        if self.generation[id.index] != id.generation {
            return None;
        }
        Some(&mut self.storage[id.index])
    }

    /// Returns `true` if `id` points to a live entry.
    pub fn is_alive(&self, id: ArenaId) -> bool {
        if id.index >= self.storage.len() {
            return false;
        }
        if self.generation[id.index] != id.generation {
            return false;
        }
        self.alive[id.index]
    }

    /// Number of live entries.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }

    /// Maximum number of slots (including both live and dead/available).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Defragment the arena, moving all live entries to a contiguous block at
    /// the front.
    ///
    /// Returns a `Vec` of `(old_id, new_id)` pairs so that external references
    /// can be updated.
    pub fn compact(&mut self) -> Vec<(ArenaId, ArenaId)> {
        let n = self.storage.len();
        let mut write_pos = 0usize;
        let mut remap = Vec::new();

        for read_pos in 0..n {
            if !self.alive[read_pos] {
                continue;
            }

            if read_pos != write_pos {
                // Move the value
                self.storage.swap(read_pos, write_pos);
                // Copy metadata
                self.generation[write_pos] = self.generation[read_pos];
                self.alive[write_pos] = true;
                // Clear old slot
                self.generation[read_pos] = self.generation[read_pos].wrapping_add(1);
                self.alive[read_pos] = false;
                self.storage[read_pos] = T::default();

                let old_id = ArenaId::new(read_pos, self.generation[write_pos]);
                let new_id = ArenaId::new(write_pos, self.generation[write_pos]);
                remap.push((old_id, new_id));
            }

            write_pos += 1;
        }

        // Clear any leftover free-list entries that point beyond active area
        self.free_list.retain(|&i| i < write_pos);
        // The remaining slots after write_pos are now implicitly free
        // Add any gaps that were left by dead entries past the live block
        let _alive_count = self.alive_count();
        // Remove stale free-list entries that overlap with live entries
        self.free_list.retain(|&i| !self.alive[i]);

        remap
    }
}

impl<T: Default + Clone> Default for Arena<T> {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

// ---------------------------------------------------------------------------
// SlotMap
// ---------------------------------------------------------------------------

/// An ergonomic wrapper around [`Arena`] that provides a slot-map-style API.
pub struct SlotMap<T: Default + Clone> {
    arena: Arena<T>,
}

impl<T: Default + Clone> SlotMap<T> {
    /// Create a new `SlotMap` with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            arena: Arena::new(capacity),
        }
    }

    /// Insert a value, returning its `ArenaId`.
    pub fn insert(&mut self, value: T) -> Option<ArenaId> {
        self.arena.alloc(value)
    }

    /// Remove the entry at `id`.  Returns `false` if `id` was already dead or
    /// stale.
    pub fn remove(&mut self, id: ArenaId) -> bool {
        self.arena.dealloc(id)
    }

    /// Borrow the value at `id` (generation-checked).
    pub fn get(&self, id: ArenaId) -> Option<&T> {
        self.arena.get(id)
    }

    /// Mutably borrow the value at `id` (generation-checked).
    pub fn get_mut(&mut self, id: ArenaId) -> Option<&mut T> {
        self.arena.get_mut(id)
    }

    /// Iterate over all alive entries as `(ArenaId, &T)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (ArenaId, &T)> {
        let n = self.arena.storage.len();
        let storage = &self.arena.storage;
        let generation = &self.arena.generation;
        let alive = &self.arena.alive;

        (0..n).filter_map(move |i| {
            if alive[i] {
                Some((ArenaId::new(i, generation[i]), &storage[i]))
            } else {
                None
            }
        })
    }

    /// Keep only entries for which `f` returns `true`.
    pub fn retain(&mut self, mut f: impl FnMut(ArenaId, &T) -> bool) {
        let n = self.arena.storage.len();
        let mut i = 0;
        while i < n {
            if self.arena.alive[i] {
                let id = ArenaId::new(i, self.arena.generation[i]);
                // Safety: we're reading through the alive check, no aliasing
                let value_ref: &T = &self.arena.storage[i];
                if !f(id, value_ref) {
                    self.arena.dealloc(id);
                    // Dealloc already bumped generation and set alive=false;
                    // continue without incrementing i because the free list
                    // slot will be consumed on next alloc, but we still need
                    // to check the same index (the loop naturally continues)
                }
            }
            i += 1;
        }
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.arena.alive_count()
    }

    /// Returns `true` if the slot map is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Default + Clone> Default for SlotMap<T> {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

// ---------------------------------------------------------------------------
// EntitySlot
// ---------------------------------------------------------------------------

/// A simple game-entity component slot.
#[derive(Debug, Clone, Default)]
pub struct EntitySlot {
    pub component_mask: u64,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Type Aliases
// ---------------------------------------------------------------------------

pub type EntityArena = Arena<EntitySlot>;
pub type VibeArena = Arena<f64>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Arena basics -----------------------------------------------------

    #[test]
    fn alloc_returns_valid_id() {
        let mut arena = Arena::<u32>::new(10);
        let id = arena.alloc(42).expect("alloc should succeed");
        assert_eq!(arena.get(id), Some(&42));
    }

    #[test]
    fn dealloc_frees_and_stale_id_returns_none() {
        let mut arena = Arena::<u32>::new(10);
        let id = arena.alloc(100).expect("alloc");
        assert!(arena.dealloc(id));
        // Same id should now be stale (generation bumped)
        assert_eq!(arena.get(id), None);
        // Double dealloc returns false
        assert!(!arena.dealloc(id));
    }

    #[test]
    fn generation_mismatch_returns_none() {
        let mut arena = Arena::<u32>::new(10);
        let id = arena.alloc(1).expect("alloc");
        let stale_id = ArenaId::new(id.index, id.generation + 1);
        assert_eq!(arena.get(stale_id), None);
        assert_eq!(arena.get_mut(stale_id), None);
        assert!(!arena.is_alive(stale_id));
    }

    #[test]
    fn capacity_limit_returns_none() {
        let mut arena = Arena::<u32>::new(3);
        assert!(arena.alloc(1).is_some());
        assert!(arena.alloc(2).is_some());
        assert!(arena.alloc(3).is_some());
        assert!(arena.alloc(4).is_none()); // full
    }

    #[test]
    fn double_dealloc_returns_false() {
        let mut arena = Arena::<u32>::new(5);
        let id = arena.alloc(10).expect("alloc");
        assert!(arena.dealloc(id));
        assert!(!arena.dealloc(id)); // already freed
    }

    #[test]
    fn alloc_reuses_free_list_slots() {
        let mut arena = Arena::<u32>::new(5);
        let a = arena.alloc(1).unwrap();
        let _b = arena.alloc(2).unwrap();
        arena.dealloc(a);
        let c = arena.alloc(3).unwrap();
        // freed slot should be reused
        assert_eq!(c.index, a.index);
        assert_eq!(c.generation, a.generation + 1);
    }

    #[test]
    fn is_alive_works() {
        let mut arena = Arena::<u32>::new(5);
        let id = arena.alloc(7).unwrap();
        assert!(arena.is_alive(id));
        arena.dealloc(id);
        assert!(!arena.is_alive(id));
    }

    #[test]
    fn alive_count_tracks_live_entries() {
        let mut arena = Arena::<u32>::new(10);
        assert_eq!(arena.alive_count(), 0);
        let a = arena.alloc(1).unwrap();
        let b = arena.alloc(2).unwrap();
        assert_eq!(arena.alive_count(), 2);
        arena.dealloc(a);
        assert_eq!(arena.alive_count(), 1);
        arena.dealloc(b);
        assert_eq!(arena.alive_count(), 0);
    }

    #[test]
    fn capacity_returns_initial_capacity() {
        let arena = Arena::<u32>::new(64);
        assert_eq!(arena.capacity(), 64);
    }

    #[test]
    fn get_mut_allows_mutation() {
        let mut arena = Arena::<u32>::new(5);
        let id = arena.alloc(0).unwrap();
        *arena.get_mut(id).unwrap() = 99;
        assert_eq!(arena.get(id), Some(&99));
    }

    #[test]
    fn dealloc_out_of_bounds_id_returns_false() {
        let mut arena = Arena::<u32>::new(5);
        let bad = ArenaId::new(999, 0);
        assert!(!arena.dealloc(bad));
    }

    // ---- compact ----------------------------------------------------------

    #[test]
    fn compact_moves_entries_and_updates_ids() {
        let mut arena = Arena::<u32>::new(10);
        let _a = arena.alloc(10).unwrap(); // index 0
        let b = arena.alloc(20).unwrap(); // index 1
        let _c = arena.alloc(30).unwrap(); // index 2

        arena.dealloc(b); // free index 1

        let remap = arena.compact();
        // After compact: a stays at 0, c moves from 2 → 1
        assert_eq!(arena.alive_count(), 2);
        // c should now be at index 1
        let moved = remap.iter().find(|(old, _)| old.index == 2);
        assert!(moved.is_some());
        let (old_id, new_id) = moved.unwrap();
        assert_eq!(new_id.index, 1);
        assert_eq!(arena.get(*new_id), Some(&30));
        assert_eq!(arena.get(*old_id), None); // old location dead
    }

    #[test]
    fn compact_all_alive_no_moves() {
        let mut arena = Arena::<u32>::new(5);
        arena.alloc(1).unwrap();
        arena.alloc(2).unwrap();
        let remap = arena.compact();
        assert!(remap.is_empty());
        assert_eq!(arena.alive_count(), 2);
    }

    #[test]
    fn compact_all_dead_clears() {
        let mut arena = Arena::<u32>::new(5);
        let a = arena.alloc(1).unwrap();
        let b = arena.alloc(2).unwrap();
        arena.dealloc(a);
        arena.dealloc(b);
        let remap = arena.compact();
        assert!(remap.is_empty());
        assert_eq!(arena.alive_count(), 0);
    }

    // ---- SlotMap ----------------------------------------------------------

    #[test]
    fn slotmap_insert_get_remove() {
        let mut sm: SlotMap<u32> = SlotMap::new(10);
        let id = sm.insert(42).unwrap();
        assert_eq!(sm.get(id), Some(&42));
        assert!(sm.remove(id));
        assert_eq!(sm.get(id), None);
    }

    #[test]
    fn slotmap_iter_skips_dead_entries() {
        let mut sm: SlotMap<u32> = SlotMap::new(10);
        sm.insert(10).unwrap();
        let id2 = sm.insert(20).unwrap();
        sm.insert(30).unwrap();
        sm.remove(id2);

        let entries: Vec<_> = sm.iter().collect();
        assert_eq!(entries.len(), 2);
        // ids may or may not be the original positions due to reuse
        for (_id, val) in &entries {
            assert!(**val == 10 || **val == 30);
        }
    }

    #[test]
    fn slotmap_retain_removes_correct_entries() {
        let mut sm: SlotMap<u32> = SlotMap::new(10);
        sm.insert(1).unwrap();
        sm.insert(2).unwrap();
        sm.insert(3).unwrap();
        sm.insert(4).unwrap();

        sm.retain(|_id, val| *val % 2 == 0);

        let entries: Vec<_> = sm.iter().collect();
        assert_eq!(entries.len(), 2);
        for (_id, val) in &entries {
            assert!(**val % 2 == 0);
        }
    }

    #[test]
    fn slotmap_len() {
        let mut sm: SlotMap<u32> = SlotMap::new(10);
        assert!(sm.is_empty());
        assert_eq!(sm.len(), 0);
        sm.insert(1).unwrap();
        sm.insert(2).unwrap();
        assert_eq!(sm.len(), 2);
    }

    #[test]
    fn slotmap_is_empty() {
        let sm: SlotMap<u32> = SlotMap::new(5);
        assert!(sm.is_empty());
    }

    // ---- EntitySlot arena -------------------------------------------------

    #[test]
    fn entity_arena_alloc_and_dealloc() {
        let mut arena: EntityArena = Arena::new(10);
        let slot = EntitySlot {
            component_mask: 0b101,
            active: true,
        };
        let id = arena.alloc(slot).unwrap();
        let stored = arena.get(id).unwrap();
        assert_eq!(stored.component_mask, 0b101);
        assert!(stored.active);
        assert!(arena.dealloc(id));
        assert!(!arena.is_alive(id));
    }

    // ---- VibeArena --------------------------------------------------------

    #[test]
    fn vibe_arena_works() {
        let mut arena: VibeArena = Arena::new(10);
        let id = arena.alloc(42.5).unwrap();
        assert_eq!(arena.get(id), Some(&42.5));
        *arena.get_mut(id).unwrap() = 99.9;
        assert_eq!(arena.get(id), Some(&99.9));
    }

    // ---- Stress test ------------------------------------------------------

    #[test]
    fn stress_fill_drain_repeat() {
        let mut arena = Arena::<u32>::new(32);
        for cycle in 0..50 {
            let mut ids = Vec::new();
            for i in 0..32 {
                let val = (cycle * 32 + i) as u32;
                let id = arena.alloc(val).expect("alloc should succeed");
                assert_eq!(arena.get(id), Some(&val));
                ids.push((id, val));
            }
            // Arena is full now
            assert!(arena.alloc(99).is_none());

            for (id, val) in &ids {
                assert!(arena.is_alive(*id));
                assert_eq!(arena.get(*id), Some(val));
            }

            for (id, _) in &ids {
                arena.dealloc(*id);
            }
            assert_eq!(arena.alive_count(), 0);
        }
    }

    // ---- Edge cases -------------------------------------------------------

    #[test]
    fn alloc_after_full_with_free_slot() {
        let mut arena = Arena::<u32>::new(3);
        let a = arena.alloc(1).unwrap();
        let _b = arena.alloc(2).unwrap();
        let _c = arena.alloc(3).unwrap();
        assert!(arena.alloc(4).is_none()); // full
        arena.dealloc(a);
        // Now should have room via free list
        let d = arena.alloc(5).unwrap();
        assert_eq!(arena.get(d), Some(&5));
    }

    #[test]
    fn default_capacity() {
        let arena = Arena::<u32>::default();
        // should be at least 1
        assert!(arena.capacity() >= 1);
    }

    #[test]
    fn zero_capacity_uses_minimum_of_one() {
        let arena = Arena::<u32>::new(0);
        assert_eq!(arena.capacity(), 1);
    }

    #[test]
    fn stale_id_after_reuse() {
        let mut arena = Arena::<u32>::new(10);
        let id = arena.alloc(100).unwrap();
        arena.dealloc(id);
        // Reuse the slot with a new value
        let id2 = arena.alloc(200).unwrap();
        assert_eq!(id2.index, id.index);
        assert_ne!(id2.generation, id.generation);
        // Old id should be dead
        assert!(!arena.is_alive(id));
        assert_eq!(arena.get(id), None);
        // New id works
        assert_eq!(arena.get(id2), Some(&200));
    }

    #[test]
    fn arena_default() {
        let arena = Arena::<u32>::default();
        assert!(arena.capacity() >= 1);
    }

    #[test]
    fn slotmap_default() {
        let sm: SlotMap<u32> = SlotMap::default();
        assert!(sm.is_empty());
    }
}
