"""JSON-RPC 2.0 stdio server and request dispatcher for Akatsuki MCP."""

import json
import sys

from akatsuki.constants import VERSION
from akatsuki.mcp.resources import MCP_RESOURCES, handle_mcp_resource_read
from akatsuki.mcp.tools import MCP_TOOLS, handle_mcp_call


def dispatch_single_request(req: dict) -> dict | None:
    """Dispatch a single JSON-RPC 2.0 request or notification dictionary."""
    if not isinstance(req, dict):
        return {
            "jsonrpc": "2.0",
            "id": None,
            "error": {"code": -32600, "message": "Invalid Request: Expected JSON object"},
        }

    is_notification = "id" not in req
    req_id = req.get("id")
    method = req.get("method")
    raw_params = req.get("params")
    params = raw_params if isinstance(raw_params, dict) else {}

    if method == "initialize":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {"subscribe": False, "listChanged": False},
                },
                "serverInfo": {"name": "akatsuki", "version": VERSION},
            },
        }
    elif method == "notifications/initialized":
        return None
    elif method == "ping":
        if is_notification:
            return None
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}
    elif method == "tools/list":
        if is_notification:
            return None
        return {"jsonrpc": "2.0", "id": req_id, "result": {"tools": MCP_TOOLS}}
    elif method == "resources/list":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"resources": MCP_RESOURCES},
        }
    elif method == "resources/templates/list":
        if is_notification:
            return None
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "resourceTemplates": [
                    {
                        "uriTemplate": "akatsuki://{note}",
                        "name": "Akatsuki Note",
                        "description": "Read any note, system spec, or project contract in the vault by stem or relative path.",
                        "mimeType": "text/markdown",
                    }
                ]
            },
        }
    elif method == "resources/read":
        if is_notification:
            return None
        uri = params.get("uri", "")
        try:
            text_out, is_err = handle_mcp_resource_read(uri)
        except Exception as e:
            text_out, is_err = f"Error reading resource '{uri}': {e!s}", True
        mime = "application/json" if uri in ("akatsuki://services", "akatsuki://projects") else "text/markdown"
        if is_err:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32602, "message": text_out},
            }
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"contents": [{"uri": uri, "mimeType": mime, "text": text_out}]},
        }
    elif method == "tools/call":
        tool_name = params.get("name")
        tool_args = params.get("arguments", {})
        if not isinstance(tool_args, dict):
            tool_args = {}
        try:
            text_out, is_err = handle_mcp_call(tool_name, tool_args)
            if is_notification:
                return None
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": text_out}],
                    "isError": is_err,
                },
            }
        except Exception as e:
            if is_notification:
                return None
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": f"Exception in {tool_name}: {e!s}",
                        }
                    ],
                    "isError": True,
                },
            }
    elif not is_notification:
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"Method not found: {method}"},
        }
    return None


def run_mcp_server():
    """Run JSON-RPC 2.0 stdio loop supporting batch and single frames."""
    sys.stderr.write("akatsuki MCP server running on stdio\n")
    sys.stderr.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception:
            resp = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": "Parse error: Invalid JSON"},
            }
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()
            continue

        if isinstance(req, list):
            if not req:
                resp = {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32600, "message": "Invalid Request: Empty batch array"},
                }
                sys.stdout.write(json.dumps(resp) + "\n")
                sys.stdout.flush()
                continue
            batch_resps = []
            for item in req:
                try:
                    single_resp = dispatch_single_request(item)
                    if single_resp is not None:
                        batch_resps.append(single_resp)
                except Exception as e:
                    item_id = item.get("id") if isinstance(item, dict) else None
                    batch_resps.append(
                        {
                            "jsonrpc": "2.0",
                            "id": item_id,
                            "error": {"code": -32603, "message": f"Internal server error: {e}"},
                        }
                    )
            if batch_resps:
                sys.stdout.write(json.dumps(batch_resps) + "\n")
                sys.stdout.flush()
        else:
            resp = dispatch_single_request(req)
            if resp is not None:
                sys.stdout.write(json.dumps(resp) + "\n")
                sys.stdout.flush()
