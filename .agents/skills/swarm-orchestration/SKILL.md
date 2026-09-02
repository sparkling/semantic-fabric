---
name: swarm-orchestration
version: "1.0.0"
author: rUv
tags: [swarm, orchestration]
description: >
  Multi-agent swarm coordination for complex tasks. Uses hierarchical topology with specialized agents to break down and execute complex work across multiple files and modules.
  Use when: 3+ files need changes, new feature implementation, cross-module refactoring, API changes with tests, security-related changes, performance optimization across codebase, database schema changes.
  Skip when: single file edits, simple bug fixes (1-2 lines), documentation updates, configuration changes, quick exploration.
---

# Swarm Orchestration Skill

## Purpose
Multi-agent swarm coordination for complex tasks. Uses hierarchical topology with specialized agents to break down and execute complex work across multiple files and modules.

## When to Trigger
- 3+ files need changes
- new feature implementation
- cross-module refactoring
- API changes with tests
- security-related changes
- performance optimization across codebase
- database schema changes

## When to Skip
- single file edits
- simple bug fixes (1-2 lines)
- documentation updates
- configuration changes
- quick exploration

<!-- ruflo-source-patch (ruvnet/ruflo#3153):swarm-orchestration -->
## Structured Interface

Choose topology from dependencies, shared-state risk, and evidence needs—not a file
count. Discover schemas, then use `swarm_init`, `hooks_route`, and
`swarm_status`. A Ruflo agent record does not launch a native host agent: register
tracked workers with `agent_spawn`, then launch matching Claude/Codex executors
separately with isolated ownership.


## References

| Document | Path | Description |
|----------|------|-------------|
| `Agent Types` | `docs/agents.md` | Complete list of agent types and capabilities |
| `Topology Guide` | `docs/topology.md` | Swarm topology configuration guide |

## Best Practices
1. Check memory for existing patterns before starting
2. Use hierarchical topology for coordination
3. Store successful patterns after completion
4. Document any new learnings
