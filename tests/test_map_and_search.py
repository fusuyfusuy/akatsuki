import shutil
import tempfile
import unittest
from pathlib import Path

from akatsuki.core import (
    dispatch_single_request,
    search_vault,
    traverse_graph,
    write_note,
)


class TestMapAndSearch(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.vault = Path(self.test_dir)
        (self.vault / "00-Meta").mkdir(parents=True, exist_ok=True)
        (self.vault / "01-Daily").mkdir(parents=True, exist_ok=True)
        (self.vault / "20-Projects").mkdir(parents=True, exist_ok=True)
        (self.vault / "40-Systems").mkdir(parents=True, exist_ok=True)

        # Setup Index and MOCs
        (self.vault / "INDEX.md").write_text(
            "---\ntitle: Index\ntype: moc\nsummary: Root Index\n---\n# Index\n[[Projects-MOC]]\n[[Systems-MOC]]\n",
            encoding="utf-8",
        )
        (self.vault / "20-Projects" / "Projects-MOC.md").write_text(
            "---\ntitle: Projects MOC\ntype: moc\nsummary: Projects\n---\n# Projects MOC\n[[node_a]]\n[[node_b]]\n[[node_c]]\n[[node_d]]\n[[node_e]]\n",
            encoding="utf-8",
        )
        (self.vault / "40-Systems" / "Systems-MOC.md").write_text(
            "---\ntitle: Systems MOC\ntype: moc\nsummary: Systems\n---\n# Systems MOC\n[[svc_web]]\n",
            encoding="utf-8",
        )

        # Cyclic and Chained Graph:
        # node_a -> node_b, node_c, svc_web
        # node_b -> node_c, node_d
        # node_c -> node_a (cycle back to A!)
        # node_d -> node_e
        write_note(
            self.vault,
            "20-Projects/node_a.md",
            "---\ntitle: Node Alpha\ntype: project\nstatus: live\nsummary: Node Alpha entrypoint\n---\n# Node Alpha\n[[node_b]]\n[[node_c]]\n[[svc_web]]\n",
        )
        write_note(
            self.vault,
            "20-Projects/node_b.md",
            "---\ntitle: Node Beta\ntype: project\nstatus: live\nsummary: Node Beta processor\n---\n# Node Beta\n[[node_c]]\n[[node_d]]\n",
        )
        write_note(
            self.vault,
            "20-Projects/node_c.md",
            "---\ntitle: Node Gamma\ntype: project\nstatus: live\nsummary: Node Gamma relayer\n---\n# Node Gamma\n[[node_a]]\n",
        )
        write_note(
            self.vault,
            "20-Projects/node_d.md",
            "---\ntitle: Node Delta\ntype: project\nstatus: live\nsummary: Node Delta worker\n---\n# Node Delta\n[[node_e]]\n",
        )
        write_note(
            self.vault,
            "20-Projects/node_e.md",
            "---\ntitle: Node Epsilon\ntype: project\nstatus: live\nsummary: Node Epsilon terminal sink\n---\n# Node Epsilon\nLeaf node.\n",
        )

        # Service boundary sink
        write_note(
            self.vault,
            "40-Systems/svc_web.md",
            "---\ntitle: Web Service\ntype: service\nstatus: live\nports: 8080\nhost: node-test\nnetwork: test-net\nsummary: Web edge service\n---\n# Web Service\nSpecification.\n",
        )

    def tearDown(self):
        shutil.rmtree(self.test_dir, ignore_errors=True)

    def test_traverse_graph_depth_1_down(self):
        out, is_err, data = traverse_graph(self.vault, "node_a", depth=1, direction="down")
        self.assertFalse(is_err)
        self.assertEqual(data["target"], "node_a")
        self.assertEqual(len(data["downstream"]), 3)
        stems = [c["stem"] for c in data["downstream"]]
        self.assertIn("node_b", stems)
        self.assertIn("node_c", stems)
        self.assertIn("svc_web", stems)
        # Depth 1 should have no children on node_b
        node_b_child = next(c for c in data["downstream"] if c["stem"] == "node_b")
        self.assertEqual(node_b_child["children"], [])
        self.assertIn("# 🗺️ Knowledge Map: `node_a`", out)

    def test_traverse_graph_depth_2_down_and_cycle(self):
        out, is_err, data = traverse_graph(self.vault, "node_a", depth=3, direction="down")
        self.assertFalse(is_err)
        # Check node_c -> node_a cycle detection
        node_c_child = next(c for c in data["downstream"] if c["stem"] == "node_c")
        self.assertTrue(len(node_c_child["children"]) > 0)
        cycle_node = node_c_child["children"][0]
        self.assertEqual(cycle_node["stem"], "node_a")
        self.assertTrue(cycle_node["cycle"])
        self.assertIn("↺ (cycle)", out)

    def test_traverse_graph_direction_up(self):
        out, is_err, data = traverse_graph(self.vault, "node_c", depth=1, direction="up")
        self.assertFalse(is_err)
        self.assertEqual(len(data["downstream"]), 0)
        self.assertTrue(len(data["upstream"]) >= 2)
        up_stems = [c["stem"] for c in data["upstream"]]
        self.assertIn("node_a", up_stems)
        self.assertIn("node_b", up_stems)
        self.assertIn("## ⬆️ Upstream Dependents", out)

    def test_traverse_graph_depth_clamping(self):
        _, _, data_min = traverse_graph(self.vault, "node_a", depth=-5)
        self.assertEqual(data_min["depth"], 1)

        _, _, data_max = traverse_graph(self.vault, "node_a", depth=99)
        self.assertEqual(data_max["depth"], 5)

    def test_traverse_graph_boundary_sinks(self):
        out, is_err, data = traverse_graph(self.vault, "node_a", depth=1, direction="both")
        self.assertFalse(is_err)
        sinks = data["boundary_sinks"]
        self.assertTrue(any(s["name"] == "svc_web" and s["ports"] == "8080" for s in sinks))
        self.assertIn("Service `svc_web`", out)
        self.assertIn("Ports: `8080`", out)

    def test_search_with_graph(self):
        # Search for Node Alpha
        results = search_vault(self.vault, "Alpha", with_graph=True)
        self.assertTrue(len(results) > 0)
        top = results[0]
        self.assertIn("graph", top)
        g = top["graph"]
        self.assertIn("downstream", g)
        self.assertIn("upstream", g)
        self.assertTrue(any("node_b" in d for d in g["downstream"]))
        self.assertTrue(any("svc_web" in d for d in g["downstream"]))

        # Search for Web Service itself
        res_svc = search_vault(self.vault, "Web", with_graph=True)
        self.assertTrue(len(res_svc) > 0)
        self.assertTrue(any("svc_web" in s for s in res_svc[0]["graph"]["services"]))

    def test_mcp_map_call(self):
        import akatsuki.core as core

        core.CURRENT_VAULT_OVERRIDE = self.vault
        try:
            req = {
                "jsonrpc": "2.0",
                "id": 101,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_map",
                    "arguments": {"target": "node_a", "depth": 2, "direction": "both"},
                },
            }
            res = dispatch_single_request(req)
            self.assertEqual(res["id"], 101)
            self.assertFalse(res["result"]["isError"])
            text = res["result"]["content"][0]["text"]
            self.assertIn("Knowledge Map: `node_a`", text)
            self.assertIn("Downstream Dependencies", text)
        finally:
            core.CURRENT_VAULT_OVERRIDE = None

    def test_mcp_search_with_graph_call(self):
        import akatsuki.core as core

        core.CURRENT_VAULT_OVERRIDE = self.vault
        try:
            req = {
                "jsonrpc": "2.0",
                "id": 102,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_search",
                    "arguments": {"query": "Alpha", "with_graph": True},
                },
            }
            res = dispatch_single_request(req)
            self.assertEqual(res["id"], 102)
            self.assertFalse(res["result"]["isError"])
            text = res["result"]["content"][0]["text"]
            self.assertIn("Downstream:", text)
            self.assertIn("Upstream:", text)
        finally:
            core.CURRENT_VAULT_OVERRIDE = None


if __name__ == "__main__":
    unittest.main()
