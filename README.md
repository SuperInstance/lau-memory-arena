# lau-memory-arena

> Pre-allocated memory pools for game entities. Generation-based IDs, zero runtime allocation on the hot path, dangling-reference protection.

## What This Does

Custom memory allocator for the **Lau (Layered Agent-UI)** game engine. Game entities are allocated from fixed-size arenas — no `malloc` during gameplay. Each entity gets a generation-based ID that detects use-after-free automatically.

The game industry calls this an "entity component system allocator." The idea: allocate all memory upfront, hand out slots, reuse slots when entities die. Generation counters catch stale references.

## Quick Start

```rust
use lau_memory_arena::{Arena, EntityId};

let mut arena: Arena<String> = Arena::new(1024); // 1024 slots

// Allocate entities
let hero: EntityId = arena.insert("Hero".into());
let npc1: EntityId = arena.insert("NPC-1".into());
let npc2: EntityId = arena.insert("NPC-2".into());

// Access by ID
let name = arena.get(hero).unwrap();
assert_eq!(name, "Hero");

// Remove (frees the slot)
arena.remove(npc1);

// Stale ID is detected via generation counter
assert!(arena.get(npc1).is_none()); // generation mismatch → None

// Slot is reused for new entities
let wolf: EntityId = arena.insert("Wolf".into()); // reuses npc1's slot
assert!(arena.get(npc1).is_none()); // old ID still invalid (generation changed)
```

## API Reference

### Arena\<T\>

| Method | Description |
|--------|-------------|
| `Arena::new(capacity)` | Pre-allocate `capacity` slots |
| `arena.insert(value)` | Allocate entity, returns `EntityId` |
| `arena.get(id)` | Borrow entity (checks generation) |
| `arena.get_mut(id)` | Mutably borrow |
| `arena.remove(id)` | Free slot (increments generation) |
| `arena.len()` | Active entities |
| `arena.capacity()` | Total slots |
| `arena.is_full()` | No free slots |
| `arena.iter()` | Iterate active entities |
| `arena.clear()` | Remove all |

### EntityId

| Method | Description |
|--------|-------------|
| `id.index()` | Slot index |
| `id.generation()` | Generation counter |
| `id.is_valid()` | Not null |

## How It Works

Each slot stores `(generation, Option<T>)`. When an entity is removed, the generation increments. When the slot is reused, the new entity gets a higher generation. Old IDs with stale generations return `None` on access — no undefined behavior, no dangling pointers.

Memory is allocated once (a `Vec` of fixed size). No runtime `malloc`/`free`. Cache-friendly sequential layout.

## Testing

28 tests: allocation, deallocation, generation-based invalidation, slot reuse, capacity limits, iteration, edge cases.

## Part of the Lau Platform

Part of the Lau game engine: [lau-git-world](https://github.com/SuperInstance/lau-git-world) · [lau-quest](https://github.com/SuperInstance/lau-quest) · [lau-biome](https://github.com/SuperInstance/lau-biome) · [lau-spatial](https://github.com/SuperInstance/lau-spatial) · [lau-audio](https://github.com/SuperInstance/lau-audio) · [lau-scheduler](https://github.com/SuperInstance/lau-scheduler) · **lau-memory-arena** · [lau-genealogy](https://github.com/SuperInstance/lau-genealogy) · [lau-recipe](https://github.com/SuperInstance/lau-recipe)

## License

MIT
