//! Model Context Protocol (MCP) native JSON-RPC 2.0 stdio server.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::path::Path;

use crate::constants::{
    MAX_MCP_RESOURCE_BYTES, MCP_DEFAULT_LIMIT, MCP_DEFAULT_READ_BUDGET, MCP_SPILL_CEILING_BYTES,
};
use crate::graph::{calculate_blast_radius, extract_contract, traverse_graph};
use crate::mutations::{
    append_section_in_note, append_work_log, read_daily_note, replace_section_in_note,
    set_note_value, write_note,
};
use crate::search::{
    execute_sql_query, format_hits_compact, get_keypath, list_notes, list_projects, list_services,
    search_vault,
};
use crate::storage::{
    contained_path, extract_section, extract_toc, read_note_utf8, resolve_note_file_unique,
};
use crate::verify::{lint_vault, reconcile_vault, run_verification_tests, verify_links};

/// Longest accepted request payload; anything larger is drained and rejected.
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

/// One JSON-RPC frame read from stdin (newline-delimited JSONL or LSP `Content-Length`).
enum Frame {
    Message {
        payload: String,
        content_length: bool,
    },
    BadUtf8,
    InvalidHeader,
    TooLarge,
}

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLarge,
}

fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<BoundedLine> {
    let mut buf: Vec<u8> = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if buf.is_empty() && !oversized {
                return Ok(BoundedLine::Eof);
            }
            return Ok(if oversized {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Line(buf)
            });
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let take = newline.map_or(chunk.len(), |i| i + 1);
        if !oversized {
            if buf.len() + take > max_bytes {
                oversized = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        reader.consume(take);
        if newline.is_some() {
            return Ok(if oversized {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Line(buf)
            });
        }
    }
}

fn is_http_header_start(trimmed: &str) -> bool {
    trimmed
        .get(..15)
        .is_some_and(|p| p.eq_ignore_ascii_case("content-length:"))
        || trimmed
            .get(..13)
            .is_some_and(|p| p.eq_ignore_ascii_case("content-type:"))
}

fn read_content_length_frame<R: BufRead>(reader: &mut R, first_line: &str) -> io::Result<Frame> {
    let mut content_len: Option<usize> = None;
    let mut current = first_line.trim_matches(['\r', '\n']).to_string();
    loop {
        if current.is_empty() {
            break;
        }
        if let Some(rest) = current
            .get(..15)
            .filter(|p| p.eq_ignore_ascii_case("content-length:"))
            .and_then(|_| current.get(15..))
        {
            match rest.trim().parse::<usize>() {
                Ok(n) => content_len = Some(n),
                Err(_) => return Ok(Frame::InvalidHeader),
            }
        }
        match read_bounded_line(reader, 8192)? {
            BoundedLine::Eof | BoundedLine::TooLarge => return Ok(Frame::InvalidHeader),
            BoundedLine::Line(bytes) => match String::from_utf8(bytes) {
                Ok(s) => current = s.trim_matches(['\r', '\n']).to_string(),
                Err(_) => return Ok(Frame::BadUtf8),
            },
        }
    }

    let Some(len) = content_len else {
        return Ok(Frame::InvalidHeader);
    };
    if len > MAX_REQUEST_BYTES {
        let _ = io::copy(&mut reader.take(len as u64), &mut io::sink())?;
        return Ok(Frame::TooLarge);
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    match String::from_utf8(body) {
        Ok(payload) => Ok(Frame::Message {
            payload,
            content_length: true,
        }),
        Err(_) => Ok(Frame::BadUtf8),
    }
}

/// Reads the next frame without ever buffering more than `MAX_REQUEST_BYTES`;
/// supports both newline-delimited JSON-RPC and `Content-Length` stdio framing.
fn read_frame<R: BufRead>(reader: &mut R) -> io::Result<Option<Frame>> {
    loop {
        match read_bounded_line(reader, MAX_REQUEST_BYTES)? {
            BoundedLine::Eof => return Ok(None),
            BoundedLine::TooLarge => return Ok(Some(Frame::TooLarge)),
            BoundedLine::Line(bytes) => match String::from_utf8(bytes) {
                Err(_) => return Ok(Some(Frame::BadUtf8)),
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if is_http_header_start(trimmed) {
                        return read_content_length_frame(reader, trimmed).map(Some);
                    }
                    return Ok(Some(Frame::Message {
                        payload: line,
                        content_length: false,
                    }));
                }
            },
        }
    }
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

/// A validated request: the id to answer, the method, and its params object.
struct Request<'a> {
    id: Value,
    method: &'a str,
    params: Value,
}

/// JSON-RPC 2.0 envelope validation. `Ok(None)` is a notification (no id): it is
/// never answered. `Err` carries the ready-made error response.
fn validate_request(req: &Value) -> Result<Option<Request<'_>>, Value> {
    let Some(object) = req.as_object() else {
        let what = if req.is_array() {
            "Batch requests are not supported"
        } else {
            "Request must be a JSON object"
        };
        return Err(rpc_error(Value::Null, -32600, what));
    };
    let Some(id) = object.get("id") else {
        return Ok(None);
    };
    if !(id.is_string() || id.is_number() || id.is_null()) {
        return Err(rpc_error(
            Value::Null,
            -32600,
            "id must be a string, number or null",
        ));
    }
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(rpc_error(id.clone(), -32600, "jsonrpc must be \"2.0\""));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Err(rpc_error(id.clone(), -32600, "method must be a string"));
    };
    let params = match object.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p) if p.is_object() => p.clone(),
        Some(_) => return Err(rpc_error(id.clone(), -32602, "params must be an object")),
    };
    Ok(Some(Request {
        id: id.clone(),
        method,
        params,
    }))
}

fn tool_exists(name: &str) -> bool {
    get_tool_definitions()
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
}

fn handle_tools_call(vault: &Path, id: Value, params: &Value) -> Value {
    let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "params.name must be a string");
    };
    if !tool_exists(tool_name) {
        return rpc_error(id, -32602, format!("Unknown tool '{}'", tool_name));
    }
    let tool_args = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(a) if a.is_object() => a.clone(),
        Some(_) => return rpc_error(id, -32602, "params.arguments must be an object"),
    };

    // A panic inside one tool must not take the whole stdio server down.
    let (text_out, is_err) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_tool(vault, tool_name, &tool_args)
    })) {
        Ok(pair) => pair,
        Err(panic) => {
            return rpc_error(
                id,
                -32603,
                format!(
                    "Internal error: tool '{}' panicked: {}",
                    tool_name,
                    panic_message(&panic)
                ),
            );
        }
    };

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text_out }],
            "isError": is_err
        }
    })
}

pub fn get_resource_definitions() -> Vec<Value> {
    vec![
        json!({
            "uri": "akatsuki://services",
            "name": "Live Services Catalog",
            "description": "Active containerized services, port allocations, and ingress routing.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "akatsuki://projects",
            "name": "Projects Inventory",
            "description": "Active production projects, tools, and technical architectures.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "akatsuki://operator",
            "name": "Operator Profile & Machine Specs",
            "description": "Hardware telemetry, communication standards, and system topology.",
            "mimeType": "text/markdown"
        }),
        json!({
            "uri": "akatsuki://daily",
            "name": "Today's Operational Log",
            "description": "Daily activity ledger, focus horizons, and session notes for today.",
            "mimeType": "text/markdown"
        }),
        json!({
            "uri": "akatsuki://invariants",
            "name": "Global Architectural Invariants",
            "description": "System-wide architectural contracts, non-negotiable boundaries, and protocols.",
            "mimeType": "text/markdown"
        }),
        json!({
            "uri": "akatsuki://index",
            "name": "Master Vault Catalog",
            "description": "Root index and map of contents across all knowledge domains.",
            "mimeType": "text/markdown"
        }),
    ]
}

pub fn get_resource_templates() -> Vec<Value> {
    vec![json!({
        "uriTemplate": "akatsuki://{note}",
        "name": "Akatsuki Vault Note",
        "description": "Read any vault note by title, stem, or relative path.",
        "mimeType": "text/markdown"
    })]
}

fn truncate_utf8_bytes(text: &str, max_bytes: usize, footer: &str) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let budget = max_bytes.saturating_sub(footer.len());
    let mut cut = budget.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &text[..cut], footer)
}

fn read_contained_anchor(vault: &Path, rel: &str) -> Result<String, String> {
    let path = contained_path(vault, rel).ok_or_else(|| format!("Path '{rel}' escapes vault."))?;
    if !path.is_file() {
        return Err(format!("{rel} not found in vault."));
    }
    read_note_utf8(&path).map_err(|e| format!("{e:#}"))
}

fn resolve_resource_payload(vault: &Path, uri: &str) -> Result<(&'static str, String), String> {
    match uri {
        "akatsuki://services" => {
            let items = list_services(vault).map_err(|e| format!("Query failed: {e}"))?;
            if !items.is_empty() {
                let text = serde_json::to_string_pretty(&items).unwrap_or_default();
                return Ok(("application/json", text));
            }
            if let Ok(cat) = read_contained_anchor(vault, "40-Systems/Services-Catalog.md") {
                return Ok(("text/markdown", cat));
            }
            Ok(("application/json", "[]".to_string()))
        }
        "akatsuki://projects" => {
            let items = list_projects(vault).map_err(|e| format!("Query failed: {e}"))?;
            if !items.is_empty() {
                let text = serde_json::to_string_pretty(&items).unwrap_or_default();
                return Ok(("application/json", text));
            }
            if let Ok(moc) = read_contained_anchor(vault, "20-Projects/Projects-MOC.md") {
                return Ok(("text/markdown", moc));
            }
            Ok(("application/json", "[]".to_string()))
        }
        "akatsuki://operator" => {
            read_contained_anchor(vault, "OPERATOR.md").map(|t| ("text/markdown", t))
        }
        "akatsuki://daily" => read_daily_note(vault, None)
            .map(|(content, _)| ("text/markdown", content))
            .map_err(|e| format!("Read daily failed: {e}")),
        "akatsuki://invariants" => {
            read_contained_anchor(vault, "AGENTS.md").map(|t| ("text/markdown", t))
        }
        "akatsuki://index" => {
            read_contained_anchor(vault, "INDEX.md").map(|t| ("text/markdown", t))
        }
        _ if uri.starts_with("akatsuki://") => {
            let target = uri.trim_start_matches("akatsuki://").trim();
            if target.is_empty() {
                return Err("Resource URI missing note target.".to_string());
            }
            let note_path =
                resolve_note_file_unique(vault, target).map_err(|e| format!("{e:#}"))?;
            let content = read_note_utf8(&note_path).map_err(|e| format!("{e:#}"))?;
            Ok(("text/markdown", content))
        }
        _ => Err(format!("Unsupported resource URI: '{uri}'")),
    }
}

fn handle_resources_read(vault: &Path, id: Value, params: &Value) -> Value {
    let Some(uri) = params
        .get("uri")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    else {
        return rpc_error(id, -32602, "params.uri must be a non-empty string");
    };

    match resolve_resource_payload(vault, uri) {
        Ok((mime, text)) => {
            let capped = truncate_utf8_bytes(
                &text,
                MAX_MCP_RESOURCE_BYTES,
                "\n\n[... Truncated at 64 KB MCP resource ceiling ...]",
            );
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "contents": [{
                        "uri": uri,
                        "mimeType": mime,
                        "text": capped
                    }]
                }
            })
        }
        Err(msg) => rpc_error(id, -32602, msg),
    }
}

fn respond(vault: &Path, request: Request<'_>) -> Value {
    let Request { id, method, params } = request;
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": { "name": "akatsuki", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {
                    "tools": {},
                    "resources": { "subscribe": false, "listChanged": false }
                }
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": get_tool_definitions() }
        }),
        "tools/call" => handle_tools_call(vault, id, &params),
        "resources/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "resources": get_resource_definitions() }
        }),
        "resources/templates/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "resourceTemplates": get_resource_templates() }
        }),
        "resources/read" => handle_resources_read(vault, id, &params),
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => rpc_error(id, -32601, format!("Method '{}' not found", method)),
    }
}

pub fn run_mcp_server(vault: &Path) -> Result<()> {
    eprintln!(
        "🌅 Akatsuki Native MCP Server listening on stdio (PID: {})...",
        std::process::id()
    );

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout();

    while let Some(frame) = read_frame(&mut stdin)? {
        let mut use_cl = false;
        let response = match frame {
            Frame::BadUtf8 => Some(rpc_error(Value::Null, -32700, "Parse error: invalid UTF-8")),
            Frame::InvalidHeader => Some(rpc_error(
                Value::Null,
                -32700,
                "Parse error: invalid Content-Length header",
            )),
            Frame::TooLarge => Some(rpc_error(
                Value::Null,
                -32600,
                format!("Request exceeds {} bytes", MAX_REQUEST_BYTES),
            )),
            Frame::Message {
                payload,
                content_length,
            } => {
                use_cl = content_length;
                if payload.trim().is_empty() {
                    None
                } else {
                    match serde_json::from_str::<Value>(payload.trim()) {
                        Err(e) => Some(rpc_error(
                            Value::Null,
                            -32700,
                            format!("Parse error: {}", e),
                        )),
                        Ok(req) => match validate_request(&req) {
                            Err(error) => Some(error),
                            Ok(None) => None,
                            Ok(Some(request)) => Some(respond(vault, request)),
                        },
                    }
                }
            }
        };
        if let Some(response) = response {
            let encoded = serde_json::to_string(&response)?;
            if use_cl {
                write!(
                    stdout,
                    "Content-Length: {}\r\n\r\n{}",
                    encoded.len(),
                    encoded
                )?;
            } else {
                writeln!(stdout, "{}", encoded)?;
            }
            stdout.flush()?;
        }
    }

    Ok(())
}

/// Unwraps a `Result<_, String>` or returns the adapter error tuple `(message, true)`.
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(m) => return (m, true),
        }
    };
}
pub fn dispatch_tool(vault: &Path, name: &str, args: &Value) -> (String, bool) {
    match name {
        "akatsuki_search" => {
            let query = tri!(req_str(args, &["query"]));
            let domain = tri!(opt_str(args, "domain"));
            let limit = tri!(arg_usize(args, "limit")).unwrap_or(MCP_DEFAULT_LIMIT);
            let with_graph = tri!(arg_bool(args, "with_graph")).unwrap_or(false);

            // Hybrid is the default intent; degradation is explicitly surfaced.
            let mode = tri!(opt_str(args, "mode")).unwrap_or("hybrid");

            match search_vault(vault, query, domain, limit, with_graph, mode) {
                Ok(hits) => {
                    let mut text = format_hits_compact(&hits);
                    if mode != "bm25" {
                        if let Some(note) = crate::vectors::hybrid_note(vault) {
                            text.push_str(&format!("\n⚠ {}", note));
                        }
                    }
                    (text, false)
                }
                Err(e) => (format!("Search failed: {}", e), true),
            }
        }
        "akatsuki_read" => {
            // Parameter aliasing: accept note, path, or target
            let target_query = tri!(req_str(args, &["note", "path", "target"]));
            let section = tri!(opt_str(args, "section"));
            let toc = tri!(arg_bool(args, "toc")).unwrap_or(false);
            let budget = tri!(arg_usize(args, "budget"));
            if toc && section.is_some() {
                return (
                    "Invalid parameters: 'toc' and 'section' are mutually exclusive.".to_string(),
                    true,
                );
            }

            let p = match resolve_note_file_unique(vault, target_query) {
                Ok(p) => p,
                Err(e) => return (format!("{e:#}"), true),
            };
            let content = match read_note_utf8(&p) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        format!("Failed to read note '{}': {e:#}", p.display()),
                        true,
                    )
                }
            };
            if toc {
                let toc_lines = extract_toc(&content);
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(target_query);
                if toc_lines.is_empty() {
                    return (
                        format!("Table of Contents for {stem}:\n  (no headings)"),
                        false,
                    );
                }
                return (
                    format!(
                        "Table of Contents for {stem}:\n  {}",
                        toc_lines.join("\n  ")
                    ),
                    false,
                );
            }
            let selected = if let Some(sec) = section {
                match extract_section(&content, sec) {
                    Some(s) => s,
                    None => {
                        let toc_lines = extract_toc(&content);
                        let hint = if toc_lines.is_empty() {
                            "  (no headings)".to_string()
                        } else {
                            format!("  - {}", toc_lines.join("\n  - "))
                        };
                        return (
                            format!(
                                "Section '{}' not found in '{}'.\nAvailable sections:\n{}",
                                sec,
                                p.display(),
                                hint
                            ),
                            true,
                        );
                    }
                }
            } else {
                content
            };
            let effective_budget = budget.or(Some(MCP_DEFAULT_READ_BUDGET));
            (
                crate::storage::apply_token_budget(&selected, effective_budget),
                false,
            )
        }
        "akatsuki_contract" => {
            let note = tri!(req_str(args, &["note", "target", "path"]));
            match extract_contract(vault, note) {
                Ok(contract) => (
                    serde_json::to_string_pretty(&contract).unwrap_or_default(),
                    false,
                ),
                Err(e) => (format!("Contract extraction failed: {}", e), true),
            }
        }
        "akatsuki_blast" => {
            let target = tri!(req_str(args, &["target", "note", "path"]));
            match calculate_blast_radius(vault, target) {
                Ok(blast) => (
                    serde_json::to_string_pretty(&blast).unwrap_or_default(),
                    false,
                ),
                Err(e) => (format!("Blast radius failed: {}", e), true),
            }
        }
        "akatsuki_map" => {
            let depth = tri!(arg_usize(args, "depth")).unwrap_or(2);
            let direction = tri!(opt_str(args, "direction")).unwrap_or("both");
            if !matches!(direction, "both" | "up" | "down") {
                return (
                    format!("Invalid direction '{direction}': expected up, down, or both."),
                    true,
                );
            }
            let target = tri!(req_str(args, &["target", "note", "path"]));

            match traverse_graph(vault, target, depth, direction) {
                Ok(graph) => (
                    serde_json::to_string_pretty(&graph.payload).unwrap_or_default(),
                    false,
                ),
                Err(e) => (format!("Map traversal failed: {}", e), true),
            }
        }
        "akatsuki_services" => match list_services(vault) {
            Ok(items) => (
                serde_json::to_string_pretty(&items).unwrap_or_default(),
                false,
            ),
            Err(e) => (format!("Query failed: {}", e), true),
        },
        "akatsuki_projects" => match list_projects(vault) {
            Ok(items) => (
                serde_json::to_string_pretty(&items).unwrap_or_default(),
                false,
            ),
            Err(e) => (format!("Query failed: {}", e), true),
        },
        "akatsuki_record_log" => {
            let project = tri!(opt_str(args, "project")).unwrap_or("");
            let summary = tri!(req_str(args, &["summary"]));
            let device = tri!(opt_str(args, "device"));

            match append_work_log(vault, project, summary, device) {
                Ok(entry) => (format!("Recorded: {}", entry), false),
                Err(e) => (format!("Log append failed: {}", e), true),
            }
        }
        "akatsuki_write_note" => {
            let path = tri!(req_str(args, &["path"]));
            let content = tri!(req_str_allow_empty(args, "content"));
            let overwrite = tri!(arg_bool(args, "overwrite")).unwrap_or(false);
            let raw = tri!(arg_bool(args, "raw")).unwrap_or(false);

            match write_note(vault, path, content, overwrite, raw) {
                Ok(p) => (
                    format!("Note successfully written to: {}", p.display()),
                    false,
                ),
                Err(e) => (format!("Write note failed: {}", e), true),
            }
        }
        "akatsuki_verify" => match verify_links(vault) {
            Ok(rep) => (
                serde_json::to_string_pretty(&rep).unwrap_or_default(),
                !rep.passed,
            ),
            Err(e) => (format!("Verification failed: {}", e), true),
        },
        "akatsuki_lint" => match lint_vault(vault) {
            Ok(rep) => (
                serde_json::to_string_pretty(&rep).unwrap_or_default(),
                !rep.passed,
            ),
            Err(e) => (format!("Lint failed: {}", e), true),
        },
        "akatsuki_test" => {
            let note = tri!(opt_str_any(args, &["note", "target"]));
            let dry_run = tri!(arg_bool(args, "dry_run")).unwrap_or(false);
            match run_verification_tests(vault, note, dry_run) {
                Ok(rep) => {
                    let pretty = serde_json::to_string_pretty(&rep).unwrap_or_default();
                    let raw = if pretty.len() > MCP_SPILL_CEILING_BYTES {
                        serde_json::to_string(&rep).unwrap_or(pretty)
                    } else {
                        pretty
                    };
                    (
                        truncate_utf8_bytes(
                            &raw,
                            MCP_SPILL_CEILING_BYTES,
                            "\n... [truncated at 3.8 KB MCP ceiling; pass 'note' to filter]",
                        ),
                        rep.failed > 0,
                    )
                }
                Err(e) => (format!("Tests failed: {}", e), true),
            }
        }
        "akatsuki_get" => {
            let keypath = tri!(req_str(args, &["key", "keypath"]));
            match get_keypath(vault, keypath) {
                Ok(val) => (
                    serde_json::to_string_pretty(&json!({ "keypath": keypath, "value": val }))
                        .unwrap_or_default(),
                    false,
                ),
                Err(e) => (format!("Get failed: {}", e), true),
            }
        }
        "akatsuki_query" => {
            let sql = tri!(req_str(args, &["sql"]));
            match execute_sql_query(vault, sql) {
                Ok(results) => {
                    let pretty = serde_json::to_string_pretty(&results).unwrap_or_default();
                    let raw = if pretty.len() > MCP_SPILL_CEILING_BYTES {
                        serde_json::to_string(&results).unwrap_or(pretty)
                    } else {
                        pretty
                    };
                    (
                        truncate_utf8_bytes(
                            &raw,
                            MCP_SPILL_CEILING_BYTES,
                            "\n... [truncated at 3.8 KB MCP ceiling; add LIMIT to SQL query]",
                        ),
                        false,
                    )
                }
                Err(e) => (format!("Query failed: {}", e), true),
            }
        }
        "akatsuki_set" => {
            let note = tri!(req_str(args, &["note"]));
            let key = tri!(req_str(args, &["key"]));
            let Some(value) = args.get("value") else {
                return (
                    "Error: Missing required parameter 'value'.".to_string(),
                    true,
                );
            };

            match set_note_value(vault, note, key, value.clone()) {
                Ok(()) => (format!("Property '{}' set on note '{}'", key, note), false),
                Err(e) => (format!("Set property failed: {}", e), true),
            }
        }
        "akatsuki_replace_section" => {
            let note = tri!(req_str(args, &["note"]));
            let heading = tri!(req_str(args, &["heading"]));
            let content = tri!(req_str_allow_empty(args, "content"));

            match replace_section_in_note(vault, note, heading, content) {
                Ok(()) => (
                    format!("Section '{}' replaced in note '{}'", heading, note),
                    false,
                ),
                Err(e) => (format!("Replace section failed: {}", e), true),
            }
        }
        "akatsuki_append_section" => {
            let note = tri!(req_str(args, &["note"]));
            let heading = tri!(req_str(args, &["heading"]));
            let content = tri!(req_str_allow_empty(args, "content"));

            match append_section_in_note(vault, note, heading, content) {
                Ok(()) => (
                    format!("Content appended under '{}' in note '{}'", heading, note),
                    false,
                ),
                Err(e) => (format!("Append section failed: {}", e), true),
            }
        }
        "akatsuki_daily" => {
            let date = tri!(opt_str(args, "date"));
            match read_daily_note(vault, date) {
                Ok((content, _)) => (content, false),
                Err(e) => (format!("Read daily failed: {}", e), true),
            }
        }
        "akatsuki_reconcile" => {
            let dry_run = tri!(arg_bool(args, "dry_run")).unwrap_or(false);
            match reconcile_vault(vault, dry_run) {
                Ok(report) => {
                    let mut text = format!(
                        "{}Reconciliation completed in {:.2}ms ({} total, {} added, {} updated, {} deleted)",
                        if dry_run { "[DRY-RUN] " } else { "" },
                        report.sync.duration_ms,
                        report.sync.total,
                        report.sync.added,
                        report.sync.updated,
                        report.sync.deleted
                    );
                    for action in &report.actions {
                        text.push_str(&format!("\n- {}", action));
                    }
                    for err in &report.sync.parse_errors {
                        text.push_str(&format!("\n- unindexable frontmatter: {}", err));
                    }
                    if let Some(note) = &report.sync.vectors {
                        text.push_str(&format!("\n- vectors: {}", note));
                    }
                    (text, false)
                }
                Err(e) => (format!("Reconcile failed: {}", e), true),
            }
        }
        "akatsuki_list_notes" => {
            let domain = tri!(opt_str(args, "domain"));
            let limit = tri!(arg_usize(args, "limit"));
            match list_notes(vault, domain) {
                Ok(mut notes) => {
                    if let Some(max) = limit {
                        notes.truncate(max);
                    }
                    let pretty = serde_json::to_string_pretty(&notes).unwrap_or_default();
                    let raw = if pretty.len() > MCP_SPILL_CEILING_BYTES {
                        serde_json::to_string(&notes).unwrap_or(pretty)
                    } else {
                        pretty
                    };
                    (
                        truncate_utf8_bytes(
                            &raw,
                            MCP_SPILL_CEILING_BYTES,
                            "\n... [truncated at 3.8 KB MCP ceiling; pass 'domain' or 'limit' to narrow]",
                        ),
                        false,
                    )
                }
                Err(e) => (format!("List notes failed: {}", e), true),
            }
        }
        _ => (format!("Tool '{}' not implemented", name), true),
    }
}

pub fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "akatsuki_search",
            "description": "Fast hybrid/BM25 search across living architecture, systems catalog, and notes. Compact response (<=2.5 KB).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query or natural language question" },
                    "domain": { "type": "string", "description": "Optional domain (e.g. '40-Systems', '20-Projects')" },
                    "limit": { "type": "integer", "default": 5, "description": "Max results to return (default: 5)" },
                    "mode": { "type": "string", "enum": ["hybrid", "bm25", "vector"], "description": "Retrieval mode" },
                    "with_graph": { "type": "boolean", "default": false, "description": "Include upstream/downstream relations" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "akatsuki_read",
            "description": "Read a note, section, or markdown TOC outline from Akatsuki. Accepts note title, path, or target interchangeably.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note title, stem, or relative path (e.g. 'Dokploy API Guide', '40-Systems/Cluster-Topology.md')" },
                    "path": { "type": "string", "description": "Alias for note" },
                    "target": { "type": "string", "description": "Alias for note" },
                    "section": { "type": "string", "description": "Optional section heading to extract" },
                    "toc": { "type": "boolean", "default": false, "description": "Return markdown table of contents outline only" },
                    "budget": { "type": "integer", "description": "Optional token budget (~4 characters each; defaults to 900)" }
                }
            }
        }),
        json!({
            "name": "akatsuki_contract",
            "description": "Extract machine boundary contract (ports, relations, invariants, verifications), eliminating 80% narrative tokens.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Target note title, stem, or relative path" },
                    "target": { "type": "string", "description": "Alias for note" },
                    "path": { "type": "string", "description": "Alias for note" }
                }
            }
        }),
        json!({
            "name": "akatsuki_blast",
            "description": "Calculate architectural blast radius (upstream dependents, downstream dependencies, boundary sinks).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Target service, stem, or note name" },
                    "note": { "type": "string", "description": "Alias for target" },
                    "path": { "type": "string", "description": "Alias for target" }
                }
            }
        }),
        json!({
            "name": "akatsuki_map",
            "description": "Traverse knowledge graph relations up to specified depth.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Origin note stem or service name" },
                    "note": { "type": "string", "description": "Alias for target" },
                    "path": { "type": "string", "description": "Alias for target" },
                    "depth": { "type": "integer", "default": 2, "description": "Traversal depth" },
                    "direction": { "type": "string", "enum": ["both", "down", "up"], "default": "both" }
                }
            }
        }),
        json!({
            "name": "akatsuki_services",
            "description": "List all live containerized services, container prefixes, port allocations, and network boundaries.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_projects",
            "description": "List all active software project architectures, repository paths, and deployment statuses.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_record_log",
            "description": "Append a telegraphic work log entry into today's daily note under kernel lock.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Project identifier (e.g. 'web', 'toku')" },
                    "summary": { "type": "string", "description": "Telegraphic caveman summary: <verb> <target> -> <delta>; exit <N>" },
                    "device": { "type": "string", "description": "Optional device identifier" }
                },
                "required": ["summary"]
            }
        }),
        json!({
            "name": "akatsuki_write_note",
            "description": "Create or replace a note in the vault under kernel lock with instant SQLite index synchronization.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Vault-relative path (e.g. '20-Projects/my-app.md')" },
                    "content": { "type": "string", "description": "Full Markdown content with YAML frontmatter" },
                    "overwrite": { "type": "boolean", "default": false },
                    "raw": { "type": "boolean", "default": false, "description": "Store raw artifact verbatim without auto-healing frontmatter" }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "akatsuki_verify",
            "description": "Validate bidirectional wikilink closure and detect broken links or orphan notes across the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_lint",
            "description": "Statically lint notes for strict YAML frontmatter and verify no repo unit test boundary violations.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "akatsuki_test",
            "description": "Run living machine invariant tests (ports, containers, daemons) embedded in bash:verify blocks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Optional note filter" },
                    "target": { "type": "string", "description": "Alias for note" },
                    "dry_run": { "type": "boolean", "default": false }
                }
            }
        }),
        json!({
            "name": "akatsuki_get",
            "description": "O(1) exact property getter across services, entities, and note frontmatter keypaths.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Keypath to extract (e.g. 'services.filament.ports', 'entities.bountools.status', 'bountools.tags')" },
                    "keypath": { "type": "string", "description": "Alias for key" }
                }
            }
        }),
        json!({
            "name": "akatsuki_query",
            "description": "Execute read-only SQL queries directly against the internal SQLite index (notes_fts, entities, services, relations, invariants, verifications, note_vectors).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "description": "Read-only SQL query string (SELECT ...)." }
                },
                "required": ["sql"]
            }
        }),
        json!({
            "name": "akatsuki_set",
            "description": "Surgically update a frontmatter key-value property without corrupting note body.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note title, stem, or relative path" },
                    "key": { "type": "string", "description": "Frontmatter property name" },
                    "value": { "description": "New value (string, number, boolean, null, array, or object)" }
                },
                "required": ["note", "key", "value"]
            }
        }),
        json!({
            "name": "akatsuki_replace_section",
            "description": "Surgically replace the contents of a specific markdown heading within a note under kernel lock.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note title, stem, or relative path" },
                    "heading": { "type": "string", "description": "Heading section to replace" },
                    "content": { "type": "string", "description": "New replacement markdown content" }
                },
                "required": ["note", "heading", "content"]
            }
        }),
        json!({
            "name": "akatsuki_append_section",
            "description": "Atomically append markdown bullets or text under a specific heading.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note title, stem, or relative path" },
                    "heading": { "type": "string", "description": "Heading section under which to append" },
                    "content": { "type": "string", "description": "Markdown content to append" }
                },
                "required": ["note", "heading", "content"]
            }
        }),
        json!({
            "name": "akatsuki_daily",
            "description": "Read today's or specified daily horizon and work ledger.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "Optional YYYY-MM-DD date (defaults to today)" }
                }
            }
        }),
        json!({
            "name": "akatsuki_reconcile",
            "description": "Auto-reconcile unindexed notes and synchronize SQLite index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dry_run": { "type": "boolean", "default": false }
                }
            }
        }),
        json!({
            "name": "akatsuki_list_notes",
            "description": "List notes in vault with metadata and domain filtering.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "domain": { "type": "string", "description": "Optional domain filter (e.g. '20-Projects', '40-Systems')" },
                    "limit": { "type": "integer", "description": "Optional maximum number of notes to return" }
                }
            }
        }),
    ]
}

/// Optional string argument; present-but-not-a-string (or null) is an error.
fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("Invalid parameter '{key}': expected a string.")),
    }
}

/// First present non-blank key among `keys` (alias order) as an optional string.
fn opt_str_any<'a>(args: &'a Value, keys: &[&str]) -> Result<Option<&'a str>, String> {
    for key in keys {
        if let Some(v) = opt_str(args, key)? {
            if !v.trim().is_empty() {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

/// Required non-empty string argument under any of the alias `keys`.
fn req_str<'a>(args: &'a Value, keys: &[&str]) -> Result<&'a str, String> {
    match opt_str_any(args, keys)? {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err(format!("Error: Missing required parameter '{}'.", keys[0])),
    }
}

/// Required string argument whose empty value is meaningful (content bodies).
fn req_str_allow_empty<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(args, key)?.ok_or_else(|| format!("Error: Missing required parameter '{key}'."))
}

/// MCP hosts routinely send booleans as strings (`"true"`) or integers (`1`/`0`); an
/// unrecognized value is an error rather than a silent `false`.
fn arg_bool(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(1) => Ok(Some(true)),
            Some(0) => Ok(Some(false)),
            _ => Err(format!("Invalid parameter '{key}': expected a boolean.")),
        },
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Some(true)),
            "false" | "0" | "no" => Ok(Some(false)),
            _ => Err(format!("Invalid parameter '{key}': expected a boolean.")),
        },
        Some(_) => Err(format!("Invalid parameter '{key}': expected a boolean.")),
    }
}

/// Accepts non-negative integers in numeric, integral-float, or string form.
fn arg_usize(args: &Value, key: &str) -> Result<Option<usize>, String> {
    let invalid = || format!("Invalid parameter '{key}': expected a non-negative integer.");
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            if let Some(u) = n.as_u64() {
                return usize::try_from(u).map(Some).map_err(|_| invalid());
            }
            if let Some(f) = n.as_f64() {
                if f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f <= (usize::MAX as f64) {
                    return Ok(Some(f as usize));
                }
            }
            Err(invalid())
        }
        Some(Value::String(s)) => s.trim().parse::<usize>().map(Some).map_err(|_| invalid()),
        Some(_) => Err(invalid()),
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
