"""Central constants and environment flags for Akatsuki."""

VERSION = "0.1.0"

DEFAULT_EMBED_MODEL = "intfloat/multilingual-e5-small"
DEFAULT_EMBED_BATCH_SIZE = 32

DOMAIN_DIRS = [
    "00-Meta",
    "01-Daily",
    "20-Projects",
    "30-Agents",
    "40-Systems",
    "50-Configs",
    "60-Scripts",
    "90-Reference",
    "90-Database",
]

DOMAIN_MOCS = {
    "01-Daily": "01-Daily/Daily-MOC.md",
    "20-Projects": "20-Projects/Projects-MOC.md",
    "30-Agents": "30-Agents/Agents-MOC.md",
    "40-Systems": "40-Systems/Systems-MOC.md",
    "90-Reference": "90-Reference/Reference-MOC.md",
    "50-Configs": "50-Configs/Configs-MOC.md",
    "60-Scripts": "60-Scripts/Scripts-MOC.md",
}

RAW_EXTS = {
    ".yml",
    ".yaml",
    ".json",
    ".toml",
    ".sh",
    ".py",
    ".conf",
    ".sql",
    ".txt",
    ".service",
    ".timer",
    ".ini",
    ".cfg",
}

try:
    import yaml  # noqa: F401
    HAVE_PYYAML = True
except ImportError:
    HAVE_PYYAML = False

try:
    import fcntl  # noqa: F401
    HAVE_FCNTL = True
except ImportError:
    HAVE_FCNTL = False
