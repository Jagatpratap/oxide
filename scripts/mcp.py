#!/usr/bin/env python3
"""Call a tool on a running Oxide engine over MCP (development helper).

    scripts/mcp.py TOOL ['{"json": "args"}'] [--png out.png]

Prints the tool's text/JSON result; image results are written to --png (default /tmp/oxide.png).
"""
import base64, json, sys, urllib.request

URL = "http://127.0.0.1:47474/mcp"

def call(tool, args):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": tool, "arguments": args}}).encode()
    req = urllib.request.Request(URL, body, {"Content-Type": "application/json",
                                             "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())

def main():
    argv = sys.argv[1:]
    png = "/tmp/oxide.png"
    if "--png" in argv:
        i = argv.index("--png"); png = argv[i + 1]; del argv[i:i + 2]
    tool = argv[0]
    args = json.loads(argv[1]) if len(argv) > 1 else {}
    resp = call(tool, args)
    if "error" in resp:
        print("RPC error:", resp["error"]); sys.exit(1)
    res = resp["result"]
    for c in res.get("content", []):
        if c["type"] == "text":
            print(c["text"])
        elif c["type"] == "image":
            open(png, "wb").write(base64.b64decode(c["data"])); print("image ->", png)
    if res.get("isError"):
        sys.exit(1)

main()
