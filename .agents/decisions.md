# Architectural Decisions (ADRs)

## ADR-0001: Architecture Baseline
- **Status**: Accepted
- **Context**: Project initialized with mimori substrate.
- **Decision**: Adopt M2M language contract and .mimori/ cache vs .agents/ memory boundaries.
- **Consequences**: Deterministic code intelligence and verifiable technical debt tracking.

## ADR-0002: Python to Rust Migration Hardening
- **Status**: Accepted
- **Context**: The v0.2.0 Rust rewrite shipped with a dead vector pipeline (`note_vectors` never written), a UTF-8 slice panic that killed the MCP server, silent frontmatter corruption on unparseable YAML, unvalidated daily-date path traversal, MCP string booleans flipping `dry_run` to false, an unbounded invariant runner, and dropped mutation provenance/audit.
- **Decision**: Complete the vector and invariant pipelines rather than delete them (docs and memory describe them as the architecture); make every degradation explicit (`--mode vector` errors, hybrid emits a notice, reconcile reports parse errors and vector status); treat malformed frontmatter as a hard error instead of an empty-metadata fallback; unify heading lookup so readers and writers agree; stamp `updated`/`updated_by` and write an audit line on every mutation.
- **Consequences**: Vector search now demonstrably returns semantically-matched notes that BM25 misses (verified with the real e5-small weights). The projection carries a `schema_meta` version and rebuilds itself on mismatch. Keyword-only builds stay fully functional and self-describing.

## ADR-0003: In-Process Lock Discipline
- **Status**: Accepted
- **Context**: `flock` locks are per file descriptor, so acquiring `.akatsuki.lock` twice in one process blocks forever. Audit-then-mutate would have deadlocked.
- **Decision**: Public entry points acquire `VaultLock` exactly once; nested helpers (`append_work_log_inner`, `record_audit`) are explicitly lock-free and documented as such.
- **Consequences**: Nested mutation+audit+sync sequences are safe; the invariant is recorded in `.agents/memory.md`.

## ADR-0004: Derived Cache, Not Migrated Cache
- **Status**: Accepted
- **Context**: The Python implementation used `.akatsuki/index.db` + `.akatsuki/vectors.db`; the Rust rewrite uses `.akatsuki/cache.db` with a different column set.
- **Decision**: Treat the cache as a pure projection: version it in `schema_meta` and drop/rebuild on mismatch instead of migrating columns in place.
- **Consequences**: Schema changes are cheap and can never leave a half-migrated index; the cost is a full re-index after an upgrade.
