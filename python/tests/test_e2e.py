import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = REPO_ROOT / "src"


class TestAkatsukiEndToEnd(unittest.TestCase):
    """Full end-to-end integration tests simulating AI agent workflows across CLI and MCP."""

    def setUp(self):
        self.tmp_dir = tempfile.mkdtemp(prefix="akatsuki_e2e_")
        self.vault_path = Path(self.tmp_dir) / "vault"
        self.vault_path.mkdir(parents=True, exist_ok=True)
        self.env = os.environ.copy()
        self.env["PYTHONPATH"] = str(SRC_DIR)
        self.env["AKATSUKI_TESTING"] = "1"
        self.env["AKATSUKI_DISABLE_HOST_EMBED"] = "1"
        self.env["AKATSUKI_VAULT"] = str(self.vault_path)

    def tearDown(self):
        shutil.rmtree(self.tmp_dir, ignore_errors=True)

    def run_cli(self, *args, check=True):
        cmd = [sys.executable, "-m", "akatsuki", "--vault", str(self.vault_path), *args]
        proc = subprocess.run(
            cmd,
            env=self.env,
            capture_output=True,
            text=True,
            cwd=str(REPO_ROOT),
        )
        if check and proc.returncode != 0:
            raise RuntimeError(
                f"CLI command failed: {' '.join(cmd)}\nExit: {proc.returncode}\nStdout: {proc.stdout}\nStderr: {proc.stderr}"
            )
        return proc

    def test_full_cli_lifecycle(self):
        """Test complete M2M lifecycle via CLI: init -> write -> replace -> append -> contract -> map -> blast -> test -> log -> lint -> verify."""
        # 1. Initialize vault
        init_res = self.run_cli("init", str(self.vault_path))
        self.assertEqual(init_res.returncode, 0)
        self.assertTrue((self.vault_path / ".akatsuki").is_dir())
        self.assertTrue((self.vault_path / "AGENTS.md").is_file())
        self.assertTrue((self.vault_path / "INDEX.md").is_file())
        self.assertTrue((self.vault_path / "20-Projects" / "Projects-MOC.md").is_file())
        self.assertTrue((self.vault_path / "40-Systems" / "Systems-MOC.md").is_file())

        # 2. Write service note
        auth_service_content = (
            "---\n"
            "type: system\n"
            "status: active\n"
            "ports:\n"
            "  - 8080\n"
            "invariants:\n"
            '  - "auth tokens must be signed with ed25519"\n'
            "---\n"
            "# auth-service\n\n"
            "## Architecture\n"
            "Core authentication and identity service handling JWT issuance.\n\n"
            "## Verification\n"
            "```bash:verify\n"
            'echo "auth-service-healthy"\n'
            "```\n"
        )
        write_res = self.run_cli(
            "write",
            "40-Systems/auth-service.md",
            "-c",
            auth_service_content,
            "--json",
        )
        self.assertEqual(write_res.returncode, 0)
        w_json = json.loads(write_res.stdout)
        self.assertEqual(w_json.get("status"), "ok")
        self.assertTrue((self.vault_path / "40-Systems" / "auth-service.md").is_file())

        # 3. Write project note referencing auth-service
        api_gateway_content = (
            "---\n"
            "type: project\n"
            "status: active\n"
            "ports:\n"
            "  - 80\n"
            "---\n"
            "# api-gateway\n\n"
            "## Overview\n"
            "Main ingress gateway connecting to [[auth-service]].\n\n"
            "## Architecture\n"
            "Legacy proxy architecture.\n"
        )
        self.run_cli(
            "write",
            "20-Projects/api-gateway.md",
            "-c",
            api_gateway_content,
            "--json",
        )
        self.assertTrue((self.vault_path / "20-Projects" / "api-gateway.md").is_file())

        # 4. Search --json and --compact
        search_res = self.run_cli("search", "JWT", "--json")
        self.assertEqual(search_res.returncode, 0)
        hits = json.loads(search_res.stdout)
        self.assertIsInstance(hits, list)
        self.assertTrue(any("auth-service" in (h.get("rel_path") or "") for h in hits))

        compact_res = self.run_cli("search", "ingress", "--compact")
        self.assertEqual(compact_res.returncode, 0)
        self.assertIn("20-Projects/api-gateway.md", compact_res.stdout)

        # 5. Contract extraction
        contract_res = self.run_cli("contract", "40-Systems/auth-service.md", "--json")
        self.assertEqual(contract_res.returncode, 0)
        contract = json.loads(contract_res.stdout)
        self.assertEqual(contract.get("title"), "auth-service")
        self.assertEqual(contract.get("type"), "system")
        self.assertTrue(any(str(p) == "8080" for p in contract.get("ports", [])))
        self.assertTrue(
            any("tokens must be signed" in inv for inv in contract.get("invariants", []))
        )
        self.assertTrue(len(contract.get("verifications", [])) > 0)
        self.assertIn('echo "auth-service-healthy"', contract["verifications"][0])

        # 6. Surgical replacement (Phase 3 feature)
        replace_res = self.run_cli(
            "replace",
            "20-Projects/api-gateway.md",
            "-H",
            "Architecture",
            "-c",
            "Modern envoy-based gateway architecture.",
            "--json",
        )
        self.assertEqual(replace_res.returncode, 0)
        rep_json = json.loads(replace_res.stdout)
        self.assertEqual(rep_json.get("status"), "ok")
        self.assertEqual(rep_json.get("heading"), "Architecture")

        # Verify disk content has modern architecture and frontmatter remains intact
        updated_gw = (self.vault_path / "20-Projects" / "api-gateway.md").read_text(encoding="utf-8")
        self.assertIn("Modern envoy-based gateway architecture.", updated_gw)
        self.assertNotIn("Legacy proxy architecture.", updated_gw)
        self.assertIn("type: project", updated_gw)
        self.assertIn("## Overview", updated_gw)

        # 7. Append section
        append_res = self.run_cli(
            "append",
            "20-Projects/api-gateway.md",
            "-H",
            "Overview",
            "-c",
            "High throughput rate limiter integrated.",
            "--json",
        )
        self.assertEqual(append_res.returncode, 0)
        app_gw = (self.vault_path / "20-Projects" / "api-gateway.md").read_text(encoding="utf-8")
        self.assertIn("High throughput rate limiter integrated.", app_gw)

        # 8. Graph blast radius and map
        blast_res = self.run_cli("blast", "auth-service", "--json")
        self.assertEqual(blast_res.returncode, 0)
        blast_data = json.loads(blast_res.stdout)
        self.assertIn("upstream", blast_data)
        self.assertTrue(
            any("api-gateway" in u.get("source_rel", "") for u in blast_data["upstream"])
        )

        map_res = self.run_cli("map", "api-gateway", "--json")
        self.assertEqual(map_res.returncode, 0)
        map_data = json.loads(map_res.stdout)
        self.assertIn("downstream", map_data)
        self.assertTrue(any(d.get("stem") == "auth-service" for d in map_data["downstream"]))

        # 9. Property getter
        get_res = self.run_cli("get", "entities.auth-service.ports", "--json")
        self.assertEqual(get_res.returncode, 0)
        get_val = json.loads(get_res.stdout)
        self.assertEqual(get_val.get("keypath"), "entities.auth-service.ports")
        self.assertTrue(any(str(p) == "8080" for p in get_val.get("value", [])))

        # 10. SQL Query
        query_res = self.run_cli("query", "SELECT title, rel_path FROM entities WHERE type='system'", "--json")
        self.assertEqual(query_res.returncode, 0)
        query_data = json.loads(query_res.stdout)
        self.assertTrue(any(q.get("title") == "auth-service" for q in query_data))

        # 11. Verification test runner (dry-run and live)
        test_dry_res = self.run_cli("test", "40-Systems/auth-service.md", "--dry-run", "--json")
        self.assertEqual(test_dry_res.returncode, 0)
        dry_data = json.loads(test_dry_res.stdout)
        self.assertEqual(dry_data.get("dry_run"), True)
        self.assertTrue(len(dry_data.get("results", [])) > 0)
        self.assertIn("DRY RUN", dry_data["results"][0]["stdout"])

        test_live_res = self.run_cli("test", "40-Systems/auth-service.md", "--json")
        self.assertEqual(test_live_res.returncode, 0)
        live_data = json.loads(test_live_res.stdout)
        self.assertEqual(live_data.get("failed"), 0)
        self.assertEqual(live_data.get("passed"), 1)

        # 12. Deposit work log
        log_res = self.run_cli(
            "log",
            "-p",
            "api-gateway",
            "-s",
            "deploy envoy ingress -> completed; exit 0",
            "--json",
        )
        self.assertEqual(log_res.returncode, 0)
        log_json = json.loads(log_res.stdout)
        self.assertEqual(log_json.get("status"), "ok")

        # 13. Lint schema compliance
        lint_res = self.run_cli("lint", "--json")
        self.assertEqual(lint_res.returncode, 0)
        lint_data = json.loads(lint_res.stdout)
        self.assertTrue(lint_data.get("passed"))
        self.assertEqual(len(lint_data.get("errors", [])), 0)

        # 14. Reconcile vault: dry-run inspection then live auto-reconciliation
        rec_dry = self.run_cli("reconcile", "--dry-run", "--json")
        self.assertEqual(rec_dry.returncode, 0)
        rec_dry_data = json.loads(rec_dry.stdout)
        self.assertIn("report", rec_dry_data)
        self.assertTrue(rec_dry_data.get("dry_run"))

        rec_live = self.run_cli("reconcile", "--json")
        self.assertEqual(rec_live.returncode, 0)
        rec_live_data = json.loads(rec_live.stdout)
        self.assertFalse(rec_live_data.get("dry_run"))

        # 15. Verify link and graph closure across vault (proves full graph closure)
        verify_res = self.run_cli("verify", "--json")
        self.assertEqual(verify_res.returncode, 0)
        verify_data = json.loads(verify_res.stdout)
        self.assertTrue(verify_data.get("passed"))
        self.assertEqual(len(verify_data.get("issues", [])), 0)

    def test_mcp_stdio_server(self):
        """Test the MCP stdio server over JSON-RPC 2.0 frames with tools/call and resources."""
        # First initialize vault and seed test note
        self.run_cli("init", str(self.vault_path))
        note_content = (
            "---\n"
            "type: system\n"
            "status: active\n"
            "ports:\n"
            "  - 9090\n"
            "invariants:\n"
            '  - "metrics port must be isolated"\n'
            "---\n"
            "# prometheus\n\n"
            "## Config\n"
            "Default scraping config.\n\n"
            "## Verification\n"
            "```bash:verify\n"
            'echo "metrics-ok"\n'
            "```\n"
        )
        self.run_cli("write", "40-Systems/prometheus.md", "-c", note_content, "--json")

        cmd = [sys.executable, "-m", "akatsuki", "--vault", str(self.vault_path), "mcp"]
        proc = subprocess.Popen(
            cmd,
            env=self.env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=str(REPO_ROOT),
        )

        def send_rpc(msg):
            line = json.dumps(msg) + "\n"
            proc.stdin.write(line)
            proc.stdin.flush()
            resp_line = proc.stdout.readline()
            if not resp_line:
                stderr = proc.stderr.read()
                raise RuntimeError(f"MCP server closed stream unexpectedly. Stderr: {stderr}")
            return json.loads(resp_line)

        try:
            # 1. Initialize
            init_req = {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "e2e-tester", "version": "1.0"},
                },
            }
            init_resp = send_rpc(init_req)
            self.assertEqual(init_resp.get("id"), 1)
            self.assertIn("result", init_resp)
            self.assertIn("capabilities", init_resp["result"])

            # 2. Initialized notification (no response expected)
            proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
            proc.stdin.flush()

            # 3. List tools
            tools_req = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
            tools_resp = send_rpc(tools_req)
            self.assertEqual(tools_resp.get("id"), 2)
            tools = tools_resp["result"]["tools"]
            tool_names = [t["name"] for t in tools]
            expected_tools = [
                "akatsuki_search",
                "akatsuki_read",
                "akatsuki_contract",
                "akatsuki_get",
                "akatsuki_query",
                "akatsuki_blast",
                "akatsuki_map",
                "akatsuki_test",
                "akatsuki_replace_section",
                "akatsuki_append_section",
                "akatsuki_write_note",
                "akatsuki_record_log",
            ]
            for exp in expected_tools:
                self.assertIn(exp, tool_names, f"Missing MCP tool: {exp}")

            # 4. Call akatsuki_search
            search_call = {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_search",
                    "arguments": {"query": "scraping"},
                },
            }
            search_resp = send_rpc(search_call)
            self.assertEqual(search_resp.get("id"), 3)
            self.assertNotIn("error", search_resp)
            content = search_resp["result"]["content"][0]["text"]
            self.assertIn("prometheus", content)

            # 5. Call akatsuki_contract
            contract_call = {
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_contract",
                    "arguments": {"note": "40-Systems/prometheus.md"},
                },
            }
            contract_resp = send_rpc(contract_call)
            self.assertEqual(contract_resp.get("id"), 4)
            contract_text = contract_resp["result"]["content"][0]["text"]
            self.assertIn("metrics port must be isolated", contract_text)

            # 6. Call akatsuki_replace_section
            replace_call = {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_replace_section",
                    "arguments": {
                        "note": "40-Systems/prometheus.md",
                        "heading": "Config",
                        "content": "Updated high-availability scraping config.",
                    },
                },
            }
            replace_resp = send_rpc(replace_call)
            self.assertEqual(replace_resp.get("id"), 5)
            # Verify update on disk
            prom_content = (self.vault_path / "40-Systems" / "prometheus.md").read_text(encoding="utf-8")
            self.assertIn("Updated high-availability scraping config.", prom_content)

            # 7. Call akatsuki_test (dry_run)
            test_call = {
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "akatsuki_test",
                    "arguments": {
                        "target": "40-Systems/prometheus.md",
                        "dry_run": True,
                    },
                },
            }
            test_resp = send_rpc(test_call)
            self.assertEqual(test_resp.get("id"), 6)
            test_text = test_resp["result"]["content"][0]["text"]
            self.assertIn("echo \"metrics-ok\"", test_text)

            # 8. List resources
            res_req = {"jsonrpc": "2.0", "id": 7, "method": "resources/list", "params": {}}
            res_resp = send_rpc(res_req)
            self.assertEqual(res_resp.get("id"), 7)
            resources = res_resp["result"]["resources"]
            self.assertTrue(len(resources) > 0)
            res_uris = [r["uri"] for r in resources]
            self.assertIn("akatsuki://index", res_uris)
            self.assertIn("akatsuki://services", res_uris)

            # 9. Read resource
            read_res_req = {
                "jsonrpc": "2.0",
                "id": 8,
                "method": "resources/read",
                "params": {"uri": "akatsuki://index"},
            }
            read_res_resp = send_rpc(read_res_req)
            self.assertEqual(read_res_resp.get("id"), 8)
            self.assertNotIn("error", read_res_resp)
            self.assertIn("contents", read_res_resp["result"])

        finally:
            if proc.stdin and not proc.stdin.closed:
                proc.stdin.close()
            if proc.stdout and not proc.stdout.closed:
                proc.stdout.close()
            if proc.stderr and not proc.stderr.closed:
                proc.stderr.close()
            proc.terminate()
            proc.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
