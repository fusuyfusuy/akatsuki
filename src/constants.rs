//! Core constants and domain configurations for Akatsuki.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DOMAIN_DIRS: &[&str] = &[
    "01-Daily",
    "20-Projects",
    "30-Agents",
    "40-Systems",
    "50-Configs",
    "60-Scripts",
    "90-Reference",
];

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
