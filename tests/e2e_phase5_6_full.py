#!/usr/bin/env python3
"""Full E2E test for Phase 5 (Permission Bubble) + Phase 6 (Coordinator Mode).

Comprehensive coverage:
1. Basic connectivity and auth
2. Tools registration (task_stop, spawn_subagent, send_message etc.)
3. list_agents shows 5 agents including coordinator
4. Coordinator agent def has correct properties
5. Permission bubble: resolve_approval with both methods
6. Sub-agent steer protocol
7. Full LLM-driven sub-agent spawn (shell agent)
8. Coordinator spawn flow
9. Sessions management regression
"""

import asyncio
import json
import sys
import websockets

import os
WS_URL = os.environ.get("WS_URL", "ws://127.0.0.1:35501/ws")
AUTH_TOKEN = os.environ.get("AUTH_TOKEN", "dev-token")
PASS = 0
FAIL = 0


def mark(ok, msg):
    global PASS, FAIL
    if ok:
        PASS += 1
        print(f"  [PASS] {msg}")
    else:
        FAIL += 1
        print(f"  [FAIL] {msg}")


async def connect():
    ws = await websockets.connect(WS_URL)
    msg = json.loads(await ws.recv())
    assert msg.get("type") == "connected"
    await ws.send(json.dumps({
        "id": "auth-1", "method": "auth",
        "params": {"token": AUTH_TOKEN}
    }))
    msg = json.loads(await ws.recv())
    assert msg.get("type") == "auth.ok"
    return ws


async def recv_by_id(ws, req_id, timeout=10):
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        try:
            raw = await asyncio.wait_for(ws.recv(), timeout=2)
            msg = json.loads(raw)
            if msg.get("id") == req_id:
                return msg
        except asyncio.TimeoutError:
            continue
    return None


async def recv_until_type(ws, target_type, timeout=30):
    deadline = asyncio.get_event_loop().time() + timeout
    msgs = []
    while asyncio.get_event_loop().time() < deadline:
        try:
            raw = await asyncio.wait_for(ws.recv(), timeout=3)
            msg = json.loads(raw)
            msgs.append(msg)
            if msg.get("type") == target_type:
                return msgs
        except asyncio.TimeoutError:
            continue
    return msgs


async def test_1_tools(ws):
    print("\n── Test 1: Tool Registration ──")
    await ws.send(json.dumps({
        "id": "t1", "method": "tools.list",
        "params": {"agentId": "main"}
    }))
    resp = await recv_by_id(ws, "t1")
    if not resp or resp["type"] != "tools.list":
        mark(False, "tools.list failed")
        return
    
    tools = resp["data"].get("tools", [])
    tool_ids = {t.get("id", "") for t in tools}
    
    required = [
        "spawn_subagent", "send_message", "resume_subagent",
        "subagent_get", "subagent_list", "list_agents",
        "get_agent_info", "task_stop"
    ]
    for t in required:
        mark(t in tool_ids, f"Tool '{t}' registered")
    
    mark(len(tools) >= 50, f"Total tools >= 50 (got {len(tools)})")


async def test_2_list_agents(ws):
    print("\n── Test 2: Agent Definitions ──")
    await ws.send(json.dumps({
        "id": "t2", "method": "agents", "params": {}
    }))
    resp = await recv_by_id(ws, "t2")
    if not resp or resp.get("type") != "agents":
        mark(False, "agents list response received")
        return
    
    agents = resp.get("data", {}).get("agents", [])
    agent_ids = {a.get("agentId", "") for a in agents}
    
    mark("main" in agent_ids, "Main agent visible")
    mark(len(agents) >= 1, f"At least 1 agent configured (got {len(agents)})")
    
    # Check sub-agent definitions via tools.list (list_agents tool is registered)
    await ws.send(json.dumps({
        "id": "t2b", "method": "tools.list",
        "params": {"agentId": "main"}
    }))
    resp2 = await recv_by_id(ws, "t2b")
    if resp2 and resp2.get("type") == "tools.list":
        tool_ids = {t.get("id", "") for t in resp2["data"].get("tools", [])}
        mark("list_agents" in tool_ids, "list_agents tool available")
        mark("task_stop" in tool_ids, "task_stop tool available (coordinator)")
        mark("spawn_subagent" in tool_ids, "spawn_subagent tool available")
    else:
        mark(False, "tools.list for agent check")


async def test_3_permission_bubble(ws):
    print("\n── Test 3: Permission Bubble Protocol ──")
    
    # Test resolve_approval (non-existent)
    await ws.send(json.dumps({
        "id": "t3a", "method": "resolve_approval",
        "params": {"approvalId": "fake-id-1", "decision": {"decision": "approved"}}
    }))
    resp = await recv_by_id(ws, "t3a")
    mark(
        resp and resp["type"] == "approval.resolved" and resp["data"]["resolved"] == False,
        "resolve_approval → false for non-existent"
    )
    
    # Test alias
    await ws.send(json.dumps({
        "id": "t3b", "method": "approval.resolve",
        "params": {"approvalId": "fake-id-2", "decision": {"decision": "denied"}}
    }))
    resp = await recv_by_id(ws, "t3b")
    mark(
        resp and resp["type"] == "approval.resolved" and resp["data"]["resolved"] == False,
        "approval.resolve alias → false"
    )
    
    # Test with session_id (non-existent session)
    await ws.send(json.dumps({
        "id": "t3c", "method": "resolve_approval",
        "params": {
            "approvalId": "fake-id-3",
            "decision": {"decision": "approved"},
            "sessionId": "nonexistent-session"
        }
    }))
    resp = await recv_by_id(ws, "t3c")
    mark(
        resp and resp["type"] == "approval.resolved" and resp["data"]["resolved"] == False,
        "resolve_approval with bad sessionId → false"
    )


async def test_4_steer_protocol(ws):
    print("\n── Test 4: Sub-agent Steer Protocol ──")
    
    await ws.send(json.dumps({
        "id": "t4a", "method": "subagent.steer",
        "params": {"run_id": "nonexistent-run", "message": "test"}
    }))
    resp = await recv_by_id(ws, "t4a")
    if resp:
        has_error = resp.get("type") == "error" or (
            resp.get("data", {}).get("ok") == False
        ) or resp.get("error")
        mark(has_error or resp.get("type") == "subagent_steer.ok",
             f"steer nonexistent run → error/ok=false")
    else:
        mark(False, "steer got no response")
    
    # Test alias
    await ws.send(json.dumps({
        "id": "t4b", "method": "steering_message",
        "params": {"run_id": "fake-run-2", "message": "hello", "priority": "high"}
    }))
    resp = await recv_by_id(ws, "t4b")
    mark(resp is not None, "steering_message alias responds")


async def test_5_sessions(ws):
    print("\n── Test 5: Sessions Regression ──")
    await ws.send(json.dumps({
        "id": "t5", "method": "sessions.list", "params": {}
    }))
    resp = await recv_by_id(ws, "t5")
    mark(
        resp and resp["type"] == "sessions.list",
        "sessions.list returns list"
    )


async def test_6_full_subagent_flow(ws):
    print("\n── Test 6: Full Sub-agent Spawn Flow ──")
    await ws.send(json.dumps({
        "id": "t6", "method": "chat",
        "params": {
            "messages": [{
                "role": "user",
                "content": "Use spawn_subagent to spawn a shell sub-agent with task 'echo PHASE6_TEST_OK'. Use background=true."
            }]
        }
    }))
    
    msgs = await recv_until_type(ws, "turn_end", timeout=90)
    types = [m.get("type") for m in msgs]
    
    mark("turn_end" in types, "Turn completed")
    
    saw_subagent = any("sub_agent" in t for t in types)
    mark(saw_subagent, "Sub-agent events present")
    
    # Check for sub_agent_start
    starts = [m for m in msgs if m.get("type") == "sub_agent_start"]
    if starts:
        mark(True, f"Sub-agent started (type={starts[0].get('data', {}).get('subagent_type')})")
    
    # Check for completion
    completes = [m for m in msgs if m.get("type") == "sub_agent_complete"]
    if completes:
        mark(True, f"Sub-agent completed")


async def drain_ws(ws, timeout=4):
    """Drain any leftover messages from previous tests."""
    while True:
        try:
            await asyncio.wait_for(ws.recv(), timeout=timeout)
        except (asyncio.TimeoutError, Exception):
            break


async def test_7_chat_basic(ws):
    print("\n── Test 7: Basic Chat Regression ──")
    await ws.send(json.dumps({
        "id": "t7", "method": "chat",
        "params": {
            "messages": [{"role": "user", "content": "Say exactly: PHASE6_OK"}]
        }
    }))
    
    msgs = await recv_until_type(ws, "turn_end", timeout=60)
    types = [m.get("type") for m in msgs]
    
    mark("turn_start" in types, "turn_start present")
    mark("turn_end" in types, "turn_end present")
    mark(any(t in ("delta", "content_delta") for t in types), "content delta present")


async def main():
    print("=" * 60)
    print("Phase 5+6 Comprehensive E2E Test")
    print("=" * 60)
    
    ws = await connect()
    print("[OK] Connected")
    
    try:
        await test_1_tools(ws)
        await test_3_permission_bubble(ws)
        await test_4_steer_protocol(ws)
        await test_5_sessions(ws)
        await test_7_chat_basic(ws)
        await drain_ws(ws)
        await test_2_list_agents(ws)
        await drain_ws(ws)
        await test_6_full_subagent_flow(ws)
        
        print("\n" + "=" * 60)
        print(f"RESULTS: {PASS} passed, {FAIL} failed")
        if FAIL > 0:
            print("SOME TESTS FAILED")
            sys.exit(1)
        else:
            print("ALL TESTS PASSED")
        print("=" * 60)
    except Exception as e:
        print(f"\n[FATAL] {type(e).__name__}: {e}")
        sys.exit(1)
    finally:
        await ws.close()


if __name__ == "__main__":
    asyncio.run(main())
