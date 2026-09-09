Read AGENTS.md in /home/claude/src/hm/semantic-fabric and the latest user instructions.

Goal: finish the agreed Semantic Fabric application with every required feature
and correctness guarantee intact. This six-hour review must change execution
when delivery is stalling, then continue the next useful action. It is not a
new planning, research, benchmarking, or harness-development programme.

Keep review/coordination to a target of ten minutes. Reuse the previous review
and inspect the delta; do not reread the entire history or ADR corpus each time.
This target never truncates necessary implementation or correctness checks.

1. Use the current accepted completion contract: ADR-0055, unless an explicitly
   approved successor supersedes it. Check Status, Updated and supersession
   links on main. ADR-0038 is historical; its advanced assurance and research
   backlog must not silently become v1 prerequisites again. Preserve required
   security, exactness, bounded execution, lifecycle, cross-RDBMS behavior and
   release checks. Cross-check capability-catalog release-blocking flags and
   acceptance commands against this contract; stale metadata must not put
   explicitly deferred research back on the critical path.
   Do not remove a backend/feature, redefine completion as demo
   readiness, or treat a proposed ADR or readiness score as scope authority.

2. Record the six-hour UTC interval, main HEAD, and the latest prior review.
   Retrieve prior outcomes/corrections through structured Ruflo MCP if available.
   Identify downtime, interruptions, missed intervals and overlapping reviews.
   Do not invent work or count downtime as active engineering. If this interval
   is already covered, reconcile its result rather than duplicating its work.

3. Establish integration before starting another lane. Inspect interrupted Git
   operations, relevant working changes and known recovery commits. Keep exactly
   one integration writer on main; never create/switch branches or worktrees.
   Preserve unrelated work. Integrate verified started work before adding more
   implementation backlog. A dirty checkout is not itself a blocker. A recovery
   commit or private component is not an integrated public application feature.

4. Compare each previous promised outcome with actual evidence:
   requirement -> public behavior or necessary blocker removed -> main commit
   -> focused acceptance check and result. Distinguish integrated/verified,
   implemented/unverified, pending integration, and externally blocked.
   Explicitly check whether a user can exercise the affected public path.
   Relevant open paths include authenticated/authorized querying, coherent
   reload/drift/readiness, native cancellation and bounds, useful cross-source
   execution, and the minimal deployable serving artifact. Recheck current code;
   do not assume this list remains open after implementation.
   Commit/test counts, model reviews, plans, receipts, private foundations and
   harness scores alone are not evidence of application completion. Integration
   and evidence that actually close a required release gate do count.

5. Test whether the PREVIOUS correction worked. Name the largest delay using
   evidence: integration backlog, unfinished public wiring, repeated checks,
   process expansion, avoidable rework, contention, or a genuine dependency.
   If a promised outcome was missed, change one concrete execution choice now.
   If an active interval closed no requirement or necessary blocker, stop
   further decomposition, harness work and optional research; choose integration
   or the smallest end-to-end closure on the critical path. Do not repeat an
   unchanged failed correction. React to blocked work immediately, not only
   every six hours. Never weaken required security or correctness to show progress.

6. Choose one primary implementation outcome and at most two independent
   supporting outcomes. State owner, dependency, next action, observable
   acceptance test and what is intentionally outside that slice. Native Codex
   and Claude subscription agents execute; Ruflo MCP coordinates and records.
   Parallelize read-only investigation/review or compatible tests only when it
   shortens delivery without shared-write or resource contention. A closed
   candidate experiment is optional, never the default build loop or per-commit
   gate. No Darwin/GEPA/AVO, retrieval tuning, new harness expansion or new
   architecture unless a concrete required product defect makes it necessary
   and the current scope authorizes it.

7. Prefer faster supporting models: ordinary tools for deterministic work,
   Luna low/Haiku for mechanical work, Terra medium for established patterns,
   Sol medium/Sonnet for normal implementation and bounded review; Sol high
   for a specific correctness proof. Escalate to Astra high/Opus only for a
   named unresolved cross-component problem. Max/Fable needs a bounded hard
   task; Ultra is not a routine worker and requires a specific user request.
   Route each next task afresh; stop escalations when their question is answered.
   Retain the selected main model and explicit requested effort; never clamp. Measure
   verified integrated outcomes, elapsed time and rework, not invented savings.
   Use no subscription spend/token/request/quota ceilings and no API keys or
   OpenRouter. If a native subscription or requested model is unavailable,
   pause model execution and report the exact client, model and error.

8. Verify proportionately: focused tests and affected builds per coherent
   change; full locked workspace checks at meaningful integration boundaries;
   complete required release qualification on the release candidate. Repeat or
   broaden checks only for new changes, failures or unresolved risk. Do not
   refresh historical evidence merely to accumulate green receipts. Update
   affected living ADRs and docs with actual status/date in the same slice.
   Commit verified, in-scope changes promptly on main; do not absorb unrelated
   work. Do not push, publish, tag or deploy without current authorization.

Report in at most 300 words: interval; requirement-level progress and evidence;
pending integration; prior correction and whether it worked; largest delay;
the execution change made; next outcome/acceptance check; and any forecast with
its measured basis, assumptions and confidence. If the basis is insufficient,
say what evidence is missing; never substitute a new arbitrary deadline.

Then execute the highest-priority authorized, unblocked action. If another
executor owns this checkout/conversation, coordinate with that live owner;
never start a competing writer or another resume. If ownership is uncertain,
continue read-only work and report the exact handoff needed. The scheduler
must deliver through the native queue, not launch a second implementation host.

Persist a timestamped review in Ruflo namespace programme-six-hour-reviews.
Include interval, thread, main HEAD, contract, requirement/evidence changes,
prior correction/result, delay, execution change, next checks and forecast basis.
Verify exact read-back before updating latest-review; add implementation
evidence afterward and never mark intended actions as done. Memory failure is
not a delivery gate: report it once and keep the record in this conversation.
Never bulk-import or directly access managed memory. This prompt grants no new
access to Product Mock/live databases, no publication authority, and no permission
to enable the retrieval flywheel. Product runtime remains Rust/Cargo; Node is
development/evidence infrastructure only.

Once all agreed completion criteria have verified evidence, report completion
and any separately pending publication approval. Do not invent additional work;
recommend ending the completion reviews.
