---
name: security-audit
version: "1.0.0"
author: rUv
tags: [security, audit]
description: >
  Comprehensive security scanning and vulnerability detection. Includes input validation, path traversal prevention, CVE detection, and secure coding pattern enforcement.
  Use when: authentication implementation, authorization logic, payment processing, user data handling, API endpoint creation, file upload handling, database queries, external API integration.
  Skip when: read-only operations on public data, internal development tooling, static documentation, styling changes.
---

# Security Audit Skill

## Purpose
Comprehensive security scanning and vulnerability detection. Includes input validation, path traversal prevention, CVE detection, and secure coding pattern enforcement.

## When to Trigger
- authentication implementation
- authorization logic
- payment processing
- user data handling
- API endpoint creation
- file upload handling
- database queries
- external API integration

## When to Skip
- read-only operations on public data
- internal development tooling
- static documentation
- styling changes

<!-- ruflo-source-patch (ruvnet/ruflo#3153):security-audit -->
## Structured Interface

Discover specialized installed security capabilities first. Use
`aidefence_scan`, `aidefence_is_safe`, and `aidefence_has_pii` only when
they are registered and reachable. Ruflo source/dependency scanning is CLI-only:
when the Brain bridge is present, obtain exact subcommand help with
`ruvnet_cli_help`, then use `ruvnet_cli_run` with literal argv. Otherwise
inspect the installed executable's help; never guess flags or claim a scan ran.


## References

| Document | Path | Description |
|----------|------|-------------|
| `Security Checklist` | `docs/security-checklist.md` | Security review checklist |
| `OWASP Guide` | `docs/owasp-top10.md` | OWASP Top 10 mitigation guide |

## Best Practices
1. Check memory for existing patterns before starting
2. Use hierarchical topology for coordination
3. Store successful patterns after completion
4. Document any new learnings
