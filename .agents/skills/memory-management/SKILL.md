---
name: memory-management
version: "1.0.0"
author: rUv
tags: [memory, management]
description: >
  AgentDB memory system with HNSW vector search. Provides 150x-12,500x faster pattern retrieval, persistent storage, and semantic search capabilities for learning and knowledge management.
  Use when: need to store successful patterns, searching for similar solutions, semantic lookup of past work, learning from previous tasks, sharing knowledge between agents, building knowledge base.
  Skip when: no learning needed, ephemeral one-off tasks, external data sources available, read-only exploration.
---

# Memory Management Skill

## Purpose
AgentDB memory system with HNSW vector search. Provides 150x-12,500x faster pattern retrieval, persistent storage, and semantic search capabilities for learning and knowledge management.

## When to Trigger
- need to store successful patterns
- searching for similar solutions
- semantic lookup of past work
- learning from previous tasks
- sharing knowledge between agents
- building knowledge base

## When to Skip
- no learning needed
- ephemeral one-off tasks
- external data sources available
- read-only exploration

<!-- ruflo-source-patch (ruvnet/ruflo#3153):memory-management -->
## Structured Interface

Discover the live schemas, then use `memory_store` to persist validated patterns
and `memory_search` / `memory_search_unified` to recall them. Memory is optional
context, not a delivery gate. Never replace a refused managed read with raw SQL,
whole-file access, or a second npx-resolved memory driver.


## References

| Document | Path | Description |
|----------|------|-------------|
| `HNSW Guide` | `docs/hnsw.md` | HNSW vector search configuration |
| `Memory Schema` | `docs/memory-schema.md` | Memory namespace and schema reference |

## Best Practices
1. Check memory for existing patterns before starting
2. Use hierarchical topology for coordination
3. Store successful patterns after completion
4. Document any new learnings
