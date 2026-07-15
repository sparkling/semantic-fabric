@AGENTS.md

# Claude Code overlay for semantic-fabric

> **The shared, canonical instructions are in `AGENTS.md`, imported above.**
> Edit shared rules THERE: they apply to both Claude Code and Codex.
> This file carries ONLY what has no bearing on Codex. If a rule would also be
> true under Codex, it belongs in `AGENTS.md`, not here.

## Skill syntax

Claude Code invokes skills with `/skill-name`. (Codex uses `$skill-name`.)

## Agent comms

The `Agent` tool and `SendMessage` are Claude Code features; Codex has no equivalent.
Named agents coordinate by messaging each other directly, not by polling shared state.

```javascript
// ALL agents in ONE message, each knowing WHO to message next
Agent({ prompt: "Research the codebase. SendMessage findings to 'architect'.",
  subagent_type: "researcher", name: "researcher", run_in_background: true })
Agent({ prompt: "Wait for 'researcher'. Design the solution. SendMessage to 'coder'.",
  subagent_type: "system-architect", name: "architect", run_in_background: true })
Agent({ prompt: "Wait for 'architect'. Implement it. SendMessage to 'tester'.",
  subagent_type: "coder", name: "coder", run_in_background: true })
Agent({ prompt: "Wait for 'coder'. Write tests. SendMessage results to 'reviewer'.",
  subagent_type: "tester", name: "tester", run_in_background: true })
Agent({ prompt: "Wait for 'tester'. Review code quality and security.",
  subagent_type: "reviewer", name: "reviewer", run_in_background: true })

// Kick off the pipeline
SendMessage({ to: "researcher", summary: "Start", message: "[task context]" })
```

| Pattern | Flow | Use when |
|---------|------|----------|
| **Pipeline** | A to B to C to D | Sequential dependencies (feature work) |
| **Fan-out** | Lead to A, B, C, back to lead | Independent parallel work (research) |
| **Supervisor** | Lead and workers, both ways | Ongoing coordination (complex refactor) |

- ALWAYS name agents; `name: "role"` is what makes one addressable
- ALWAYS tell an agent who to message and what to send. An agent with no SendMessage
  instruction finishes and goes idle without ever reporting back
- Spawn ALL agents in ONE message with `run_in_background: true`
- After spawning: STOP, tell the user what is running, and wait
- NEVER poll status. Agents message back, or they complete

(Which agent types exist, and when a swarm is worth it at all, is in `AGENTS.md`.)

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

## Setup

```bash
claude mcp add claude-flow -- npx -y ruflo@latest mcp start
npx ruflo@latest doctor --fix
```
