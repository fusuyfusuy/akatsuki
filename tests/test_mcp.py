import shutil
import tempfile
import unittest
from pathlib import Path

import akatsuki.core as core
from akatsuki.core import dispatch_single_request, write_note


class TestAkatsukiMCP(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.test_dir = tempfile.mkdtemp()
        cls.vault = Path(cls.test_dir)
        (cls.vault / "01-Daily").mkdir(parents=True, exist_ok=True)
        (cls.vault / "20-Projects").mkdir(parents=True, exist_ok=True)
        write_note(
            cls.vault,
            "20-Projects/Dokploy-Traefik.md",
            "---\ntitle: Dokploy Traefik\ntype: project\nstatus: live\ntags: [proxy]\nsummary: Traefik edge router\n---\n# Dokploy Traefik\n\nRouting specification.\n",
        )
        core.CURRENT_VAULT_OVERRIDE = cls.vault

    @classmethod
    def tearDownClass(cls):
        core.CURRENT_VAULT_OVERRIDE = None
        shutil.rmtree(cls.test_dir, ignore_errors=True)

    def test_mcp_initialize(self):
        req = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"clientInfo": {"name": "test-client", "version": "1.0.0"}},
        }
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 1)
        self.assertIn("serverInfo", res["result"])
        self.assertEqual(res["result"]["serverInfo"]["name"], "akatsuki")

    def test_mcp_tools_list(self):
        req = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 2)
        tools = res["result"]["tools"]
        self.assertTrue(len(tools) >= 16)
        tool_names = [t["name"] for t in tools]
        self.assertIn("akatsuki_search", tool_names)
        self.assertIn("akatsuki_read", tool_names)
        self.assertIn("akatsuki_contract", tool_names)
        self.assertIn("akatsuki_test", tool_names)
        self.assertIn("akatsuki_reconcile", tool_names)

    def test_mcp_resources_list(self):
        req = {"jsonrpc": "2.0", "id": 3, "method": "resources/list", "params": {}}
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 3)
        self.assertIn("resources", res["result"])

    def test_mcp_resources_templates_list(self):
        req = {"jsonrpc": "2.0", "id": 4, "method": "resources/templates/list", "params": {}}
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 4)
        self.assertIn("resourceTemplates", res["result"])
        templates = res["result"]["resourceTemplates"]
        self.assertTrue(any("akatsuki://{note}" in t["uriTemplate"] for t in templates))

    def test_mcp_notification_silence(self):
        # Requests omitting 'id' are notifications and MUST NOT return responses
        req_notify = {"jsonrpc": "2.0", "method": "ping", "params": {}}
        res = dispatch_single_request(req_notify)
        self.assertIsNone(res)

        req_tools = {"jsonrpc": "2.0", "method": "tools/list", "params": {}}
        res = dispatch_single_request(req_tools)
        self.assertIsNone(res)

    def test_mcp_read_budget_coercion(self):
        # Verify that string budget does not raise TypeError ('<=' not supported)
        req = {
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "akatsuki_read",
                "arguments": {"note": "Dokploy-Traefik", "budget": "200"},
            },
        }
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 5)
        self.assertFalse(res["result"]["isError"])

    def test_mcp_resource_read_error_safety(self):
        req = {
            "jsonrpc": "2.0",
            "id": 6,
            "method": "resources/read",
            "params": {"uri": "akatsuki://nonexistent_spec"},
        }
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 6)
        self.assertIn("error", res)
        self.assertEqual(res["error"]["code"], -32602)

    def test_mcp_reconcile_call(self):
        req = {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "akatsuki_reconcile",
                "arguments": {"dry_run": True},
            },
        }
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 7)
        self.assertFalse(res["result"]["isError"])


if __name__ == "__main__":
    unittest.main()
