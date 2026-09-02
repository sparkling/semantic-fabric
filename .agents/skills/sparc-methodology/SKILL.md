---
name: sparc-methodology
version: "1.0.0"
author: rUv
tags: [sparc, methodology]
description: >
  SPARC development workflow: Specification, Pseudocode, Architecture, Refinement, Completion. A structured approach for complex implementations that ensures thorough planning before coding.
  Use when: new feature implementation, complex implementations, architectural changes, system redesign, integration work, unclear requirements.
  Skip when: simple bug fixes, documentation updates, configuration changes, well-defined small tasks, routine maintenance.
---

# Sparc Methodology Skill

## Purpose
SPARC development workflow: Specification, Pseudocode, Architecture, Refinement, Completion. A structured approach for complex implementations that ensures thorough planning before coding.

## When to Trigger
- new feature implementation
- complex implementations
- architectural changes
- system redesign
- integration work
- unclear requirements

## When to Skip
- simple bug fixes
- documentation updates
- configuration changes
- well-defined small tasks
- routine maintenance

<!-- ruflo-source-patch (ruvnet/ruflo#3153):sparc-methodology -->
## Structured Interface

Use native host tools to execute Specification, Pseudocode, Architecture,
Refinement, and Completion. When Ruflo routing is useful, discover and call
`hooks_route`; recall prior constraints with `memory_search`, and store only
validated reusable outcomes with `memory_store`.


## References

| Document | Path | Description |
|----------|------|-------------|
| `SPARC Overview` | `docs/sparc.md` | Complete SPARC methodology guide |
| `Phase Templates` | `docs/sparc-templates.md` | Templates for each SPARC phase |

## Best Practices
1. Check memory for existing patterns before starting
2. Use hierarchical topology for coordination
3. Store successful patterns after completion
4. Document any new learnings
