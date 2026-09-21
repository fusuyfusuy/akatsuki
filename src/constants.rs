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

pub const RAW_EXTS: &[&str] = &[
    ".conf", ".yaml", ".yml", ".json", ".toml", ".sh", ".py", ".sql", ".ini", ".env", ".service",
];

pub const DEFAULT_EMBED_MODEL: &str = "intfloat/multilingual-e5-small";
pub const DEFAULT_EMBED_DIM: usize = 384;
pub const MCP_DEFAULT_LIMIT: usize = 5;
pub const CLI_DEFAULT_LIMIT: usize = 10;
