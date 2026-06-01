# lau-memory-arena

**Pre-allocated memory pools for game entities — generation-based IDs, zero runtime allocation on the hot path, dangling-reference protection**

Custom arena allocator for the Lau game engine. Game entities live in fixed-size arenas — no `malloc` during gameplay. Each entity gets a generation-based ID that detects use-after-free automatically.

---

## What This Does

`lau-memory-arena` provides two core types:

- **`Arena<T>`** — A pre-allocated, generation-based arena allocator that stores values in a contiguous `Vec<T>`. Allocation reuses freed slots via a free list and only grows when new capacity is needed past the initial reserve. Deallocation increments the entry's generation so stale IDs become invalid.

- **`SlotMap<T>`** — An ergonomic wrapper around `Arena<T>` with a slot-map-style API including iteration and `retain`.

Plus ready-made type aliases:
- `EntityArena = Arena<EntitySlot>` — for game entities with a component mask
- `VibeArena = Arena<f64>` — for vibe/energy values

---

## Key Idea

Each slot in the arena carries a **generation counter**. When a slot is freed, its generation increments. When the slot is reused, the new occupant gets the new generation. Old IDs with stale generations return `None` on access — no undefined behavior, no dangling pointers, no `unsafe` code.

```
Alloc slot 0 → gen 0, value = "Hero"
Dealloc slot 0 → gen becomes 1, slot added to free list
Alloc again → slot 0 reused, gen 1, value = "Wolf"
Old ID (gen 0) → is_alive() = false, get() = None  ✓
New ID (gen 1) → is_alive() = true, get() = Some(&"Wolf")  ✓
```

Memory is allocated once at construction. No runtime `malloc`/`free`. Cache-friendly sequential layout.

---

## Install

```toml
[dependencies]
lau-memory-arena = "0.1.0"
```

Requires Rust 2021 edition. **Zero dependencies.**

---

## Quick Start

### Basic arena usage

```rust
use lau_memory_arena::{Arena, ArenaId};

let mut arena: Arena<u32> = Arena::new(1024); // pre-allocate 1024 slots

// Allocate entries
let id_a = arena.alloc(42).unwrap();
let id_b = arena.alloc(99).unwrap();

// Access by ID
assert_eq!(arena.get(id_a), Some(&42));
assert_eq!(arena.get(id_b), Some(&99));

// Mutate
*arena.get_mut(id_a).unwrap() = 100;
assert_eq!(arena.get(id_a), Some(&100));

// Check liveness
assert!(arena.is_alive(id_a));

// Deallocate — generation increments, slot goes to free list
assert!(arena.dealloc(id_a));
assert!(!arena.is_alive(id_a));        // stale ID
assert_eq!(arena.get(id_a), None);      // returns None

// Slot is reused for new entries
let id_c = arena.alloc(7).unwrap();
assert_eq!(id_c.index, id_a.index);     // same slot index
assert_ne!(id_c.generation, id_a.generation); // but new generation
assert_eq!(arena.get(id_c), Some(&7));  // new ID works
assert_eq!(arena.get(id_a), None);      // old ID still invalid
```

### Using SlotMap

```rust
use lau_memory_arena::SlotMap;

let mut sm: SlotMap<String> = SlotMap::new(64);

let hero = sm.insert("Hero".into()).unwrap();
let npc = sm.insert("NPC".into()).unwrap();

// Iterate alive entries
for (id, name) in sm.iter() {
    println!("{}: {}", id.index, name);
}

// Conditional removal
sm.retain(|_id, name| name != "NPC");

assert_eq!(sm.len(), 1);
assert!(sm.get(hero).is_some());
```

### Entity arenas

```rust
use lau_memory_arena::{EntityArena, EntitySlot};

let mut arena: EntityArena = Arena::new(256);

let entity = arena.alloc(EntitySlot {
    component_mask: 0b101, // has transform + physics
    active: true,
}).unwrap();

// Mutate the component mask
arena.get_mut(entity).unwrap().component_mask |= 0b1000; // add rendering
```

### Compaction (defragmentation)

```rust
// After many alloc/dealloc cycles, the arena gets fragmented.
// compact() moves all live entries to a contiguous block at the front.
let remap = arena.compact();

// remap is Vec<(ArenaId, ArenaId)> — (old_id, new_id) pairs
// Use this to update any external references
for (old_id, new_id) in &remap {
    println!("moved slot {} → {}", old_id.index, new_id.index);
}
```

---

## API Reference

### `ArenaId`

```rust
pub struct ArenaId {
    pub index: usize,       // Slot position in the arena
    pub generation: u64,    // Generation counter for this slot
}
```

Generation-based ID. Two `ArenaId`s are equal only if **both** index and generation match. A stale ID (wrong generation) will fail all access checks.

### `Arena<T: Default + Clone>`

The core arena allocator.

| Method | Signature | Description |
|---|---|---|
| `new` | `(capacity: usize) -> Self` | Pre-allocate `capacity` slots (minimum 1) |
| `alloc` | `(value: T) -> Option<ArenaId>` | Allocate a slot; prefers free list, then next sequential slot. Returns `None` if full. |
| `dealloc` | `(id: ArenaId) -> bool` | Free a slot. Increments generation, adds to free list. Returns `false` if stale or already freed. |
| `get` | `(id: ArenaId) -> Option<&T>` | Borrow the value (generation-checked). |
| `get_mut` | `(id: ArenaId) -> Option<&mut T>` | Mutably borrow the value (generation-checked). |
| `is_alive` | `(id: ArenaId) -> bool` | Returns `true` if the ID points to a live entry. |
| `alive_count` | `() -> usize` | Number of currently live entries. |
| `capacity` | `() -> usize` | Maximum number of slots (live + available). |
| `compact` | `() -> Vec<(ArenaId, ArenaId)>` | Defragment: move all live entries to the front. Returns old→new ID remap. |

Implements `Default` (capacity 1024).

### `SlotMap<T: Default + Clone>`

Ergonomic wrapper around `Arena<T>`.

| Method | Signature | Description |
|---|---|---|
| `new` | `(capacity: usize) -> Self` | Create a slot map |
| `insert` | `(value: T) -> Option<ArenaId>` | Insert a value |
| `remove` | `(id: ArenaId) -> bool` | Remove by ID |
| `get` | `(id: ArenaId) -> Option<&T>` | Borrow value |
| `get_mut` | `(id: ArenaId) -> Option<&mut T>` | Mutably borrow |
| `iter` | `() -> impl Iterator<Item = (ArenaId, &T)>` | Iterate all alive entries |
| `retain` | `(f: FnMut(ArenaId, &T) -> bool)` | Keep only entries matching predicate |
| `len` | `() -> usize` | Number of live entries |
| `is_empty` | `() -> bool` | Whether the map is empty |

Implements `Default` (capacity 1024).

### `EntitySlot`

```rust
pub struct EntitySlot {
    pub component_mask: u64,   // Bitmask of attached components
    pub active: bool,          // Whether the entity is active
}
```

Implements `Default` (mask = 0, active = false) and `Clone`.

### Type Aliases

```rust
pub type EntityArena = Arena<EntitySlot>;
pub type VibeArena = Arena<f64>;
```

---

## How It Works

### Slot layout

Internally, `Arena<T>` maintains four parallel vectors:

```
storage:   [ T::default(), T::default(), ..., T::default() ]  // capacity slots
generation: [ 0, 0, 0, ..., 0 ]                               // one u64 per slot
alive:     [ false, false, ..., false ]                        // liveness bitmap
free_list: [ ]                                                 // stack of freed slot indices
```

`storage` is pre-filled with `T::default()` values. `generation` starts at 0 for all slots. `alive` tracks which slots are currently occupied.

### Allocation path

1. **Check free list first.** If a freed slot is available, pop it, write the value, mark alive. The generation was already incremented during dealloc, so the new ID gets the current generation.

2. **Fall back to next sequential slot.** If `next_alloc < capacity`, claim `next_alloc`, write the value, mark alive. Generation is 0 (never deallocated).

3. **Return `None` if full.** No free list entries and `next_alloc >= capacity`.

### Deallocation path

1. Validate: index in bounds, generation matches, slot is alive.
2. Set `alive[index] = false`.
3. Increment `generation[index]` (wrapping_add).
4. Push index to free list.
5. Write `T::default()` to the slot (clear old data).

### Generation safety

A stale ID has `id.generation != arena.generation[id.index]`. Every access method (`get`, `get_mut`, `is_alive`, `dealloc`) checks this first. A stale ID can never accidentally reach new data.

The generation counter uses `wrapping_add(1)`, so it safely wraps around at `u64::MAX` (≈1.8 × 10¹⁹ deallocs per slot before collision — not a practical concern).

### Compaction

`compact()` performs a single-pass compaction:

1. Iterate slots from left to right.
2. For each live slot not at the current write position, swap it to the front.
3. Clear the old slot (increment generation, mark dead).
4. Return a remap table of `(old_id, new_id)` pairs.

After compaction, all live entries are contiguous starting from index 0. The free list is cleaned to remove stale entries.

---

## The Math

### Allocation as a stack machine

The free list acts as a **LIFO stack**. Alloc pops, dealloc pushes. This means recently freed slots are reused first — good for cache locality (the slot was likely accessed recently).

```
alloc():
  if free_list.nonempty():
    index = free_list.pop()     // O(1)
  else if next_alloc < capacity:
    index = next_alloc          // O(1)
    next_alloc += 1
  else:
    return None                 // arena full

  alive[index] = true
  storage[index] = value
  return ArenaId { index, generation[index] }
```

All operations are **O(1)** — no scanning, no searching.

### Generation counter as a version vector

Each slot has an independent monotonically increasing version number. An `ArenaId` is a **(index, version)** pair. Two IDs are equal iff both components match:

```
id₁ == id₂  ⟺  id₁.index == id₂.index ∧ id₁.generation == id₂.generation
```

After `n` deallocs of slot `i`, the generation is `n` (modulo wrapping). An ID from generation `k` is valid only if the current generation is exactly `k`:

```
is_alive(id) = (generation[id.index] == id.generation) ∧ alive[id.index]
```

This is equivalent to an **epoch-based reclamation** scheme, but simpler because there's only one "thread" (the arena owner).

### Space complexity

```
Arena<T> memory = capacity × (sizeof(T) + sizeof(u64) + sizeof(bool)) + free_list_overhead
```

For `Arena<u32>` with capacity 1024:
- Storage: 1024 × 4 = 4 KB
- Generation: 1024 × 8 = 8 KB
- Alive: 1024 × 1 = 1 KB
- Total: ≈ 13 KB (pre-allocated, no growth)

### Compaction complexity

`compact()` is a single pass: **O(capacity)** time, **O(alive_count)** space for the remap table. No additional allocation needed for the compaction itself — it works in-place via swaps.

---

## Testing

The crate includes **28 tests** covering:

- Basic allocation and access
- Deallocation and stale ID detection
- Generation mismatch handling
- Capacity limits and full arena behavior
- Free list slot reuse with generation bumps
- `is_alive` and `alive_count` tracking
- `get_mut` for in-place mutation
- Out-of-bounds ID handling
- `compact()` with dead entries, all-alive, and all-dead cases
- `SlotMap` insert/get/remove, iteration skipping dead entries, `retain` filtering
- `EntityArena` and `VibeArena` type aliases
- Stress test: 50 cycles of fill-all → verify → drain-all
- Edge cases: zero capacity (minimum 1), default capacity, stale ID after reuse

Run with:

```bash
cargo test
```

---

## Part of the Lau Platform

Part of the Lau game engine: [lau-git-world](https://github.com/SuperInstance/lau-git-world) · [lau-quest](https://github.com/SuperInstance/lau-quest) · [lau-biome](https://github.com/SuperInstance/lau-biome) · [lau-spatial](https://github.com/SuperInstance/lau-spatial) · [lau-audio](https://github.com/SuperInstance/lau-audio) · [lau-scheduler](https://github.com/SuperInstance/lau-scheduler) · **lau-memory-arena** · [lau-genealogy](https://github.com/SuperInstance/lau-genealogy) · [lau-recipe](https://github.com/SuperInstance/lau-recipe)

## License

MIT
