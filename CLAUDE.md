@AGENTS.md

# Claude Code overlay for semantic-fabric

> **The shared, canonical instructions are in `AGENTS.md`, imported above.**
> Edit shared rules THERE: they apply to both Claude Code and Codex.
> This file carries ONLY what has no bearing on Codex. If a rule would also be
> true under Codex, it belongs in `AGENTS.md`, not here.

## Skill syntax

Claude Code invokes skills with `/skill-name`. (Codex uses `$skill-name`.)

**ruflo-managed:claude-agents:v2**

## Agent comms

The `Agent` tool and `SendMessage` are Claude Code features; Codex uses its own native agent surface. Named native agents coordinate by messaging, not by polling shared state. Native agents are not automatically Ruflo-tracked.

- Name every agent and tell it who receives which result.
- Launch independent agents together; give writers isolated worktrees and non-overlapping ownership.
- For Ruflo-tracked work, create the structured swarm/agent records before launching matching native agents.
- After spawning, continue independent work. Wait only when a real dependency blocks progress.
- Do not poll repeatedly; agents message back or complete through the native host.

## Model routing

Claude's model lineup, so it lives here and not in `AGENTS.md`. Route by complexity, not by habit:
the cheapest tier that can do the job correctly.

| Tier | Handler | Use cases |
|------|---------|-----------|
| 1 | Agent Booster (WASM) | Mechanical transforms; skip the LLM and use an edit directly |
| 2 | Haiku | Simple, low-complexity tasks |
| 3 | Sonnet | Everyday implementation, tests, refactors |
| 4 | Opus | Architecture, security, the hardest reasoning |

## Commit attribution

The Bash tool's default commit-message template suggests a `Co-Authored-By` trailer. Ignore it.
The rule itself, and its rationale, are in `AGENTS.md`.

**ruflo-managed:claude-setup:v2**

## Setup

`ruflo-core` owns the Claude MCP server when the plugin is installed. Do not register a duplicate standalone `claude-flow` server because both would write the same project state.

Direct diagnostics remain valid:

```bash
npx ruflo@latest doctor --fix
```