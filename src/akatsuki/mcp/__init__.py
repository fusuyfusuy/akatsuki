"""Model Context Protocol (MCP) server package for Akatsuki."""

from akatsuki.mcp.resources import MCP_RESOURCES, handle_mcp_resource_read
from akatsuki.mcp.server import dispatch_single_request, run_mcp_server
from akatsuki.mcp.tools import MCP_TOOLS, handle_mcp_call

__all__ = [
    "MCP_RESOURCES",
    "MCP_TOOLS",
    "dispatch_single_request",
    "handle_mcp_call",
    "handle_mcp_resource_read",
    "run_mcp_server",
]
