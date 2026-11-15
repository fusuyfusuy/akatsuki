//! Core constants and domain configurations for Akatsuki.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Domain index notes: every note in a listed domain must be reachable from its MOC
/// or from `INDEX.md`.
pub const DOMAIN_MOCS: &[(&str, &str)] = &[
    ("01-Daily", "01-Daily/Daily-MOC.md"),
    ("20-Projects", "20-Projects/Projects-MOC.md"),
    ("30-Agents", "30-Agents/Agents-MOC.md"),
    ("40-Systems", "40-Systems/Systems-MOC.md"),
    ("50-Configs", "50-Configs/Configs-MOC.md"),
    ("60-Scripts", "60-Scripts/Scripts-MOC.md"),
    ("90-Reference", "90-Reference/Reference-MOC.md"),
];

/// Vault root anchors are never orphans.
pub const ROOT_ANCHORS: &[&str] = &["index.md", "operator.md", "agents.md", "readme.md"];

pub const RAW_EXTS: &[&str] = &[
    ".conf", ".yaml", ".yml", ".json", ".toml", ".sh", ".py", ".sql", ".ini", ".env", ".service",
];

pub const DEFAULT_EMBED_MODEL: &str = "intfloat/multilingual-e5-small";
pub const DEFAULT_EMBED_DIM: usize = 384;
pub const MCP_DEFAULT_LIMIT: usize = 5;
pub const CLI_DEFAULT_LIMIT: usize = 10;

/// Longest a mutation waits for `.akatsuki.lock` before failing loudly.
pub const LOCK_TIMEOUT_SECS: u64 = 10;

/// Hard ceiling on hits returned by any search entry point (CLI and MCP).
pub const MAX_SEARCH_LIMIT: usize = 100;

/// Wall-clock budget for one read-only SQL query before it is interrupted.
pub const SQL_DEADLINE_SECS: u64 = 5;

/// SQLite length limit for any single string/blob produced by a query.
pub const SQL_MAX_VALUE_BYTES: i32 = 8 * 1024 * 1024;

/// Maximum columns permitted in a user-supplied read-only SQL query.
pub const SQL_MAX_COLUMNS: i32 = 64;

/// Cumulative serialized byte budget across all rows of a read-only SQL query.
pub const SQL_MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

/// Maximum size of a single note file read into memory during mutations/reads.
pub const MAX_NOTE_READ_BYTES: u64 = 8 * 1024 * 1024;

/// Default token budget applied to `akatsuki_read` when MCP caller omits `budget`.
pub const MCP_DEFAULT_READ_BUDGET: usize = 900;

/// Byte ceiling for MCP list/query/test tool responses to avoid the 4 KB host spill trap.
pub const MCP_SPILL_CEILING_BYTES: usize = 3_800;

/// Byte ceiling for a single `resources/read` response over MCP stdio.
pub const MAX_MCP_RESOURCE_BYTES: usize = 65_536;
