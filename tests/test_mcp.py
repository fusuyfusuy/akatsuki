import unittest
from akatsuki.core import dispatch_single_request, MCP_TOOLS, MCP_RESOURCES


class TestAkatsukiMCP(unittest.TestCase):
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
        self.assertTrue(len(tools) >= 15)
        tool_names = [t["name"] for t in tools]
        self.assertIn("akatsuki_search", tool_names)
        self.assertIn("akatsuki_read", tool_names)
        self.assertIn("akatsuki_contract", tool_names)
        self.assertIn("akatsuki_test", tool_names)

    def test_mcp_resources_list(self):
        req = {"jsonrpc": "2.0", "id": 3, "method": "resources/list", "params": {}}
        res = dispatch_single_request(req)
        self.assertEqual(res["id"], 3)
        self.assertIn("resources", res["result"])


if __name__ == "__main__":
    unittest.main()
