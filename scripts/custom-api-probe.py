"""探测一个接口是否说 Claude（Anthropic Messages）协议（与 custom_api/verify.rs 的两步校验一致：认证 → 响应格式）。

环境变量（key 只从环境变量读，不打印、不写文件）：
  PROBE_BASE_URL   如 https://api.anthropic.com（可带 /v1 或 /v1/messages，会去掉）
  PROBE_API_KEY    接口 key；未设时读 ANTHROPIC_TEST_KEY
  PROBE_MODEL      如 claude-haiku-4-5、qwen3-max、deepseek-chat
  PROBE_AUTH       可选 x-api-key / bearer；不填则依次尝试
  PROBE_BETA       可选 anthropic-beta 头（如经订阅 OAuth 对照时用 oauth-2025-04-20）
  PROBE_SYSTEM     可选 system 提示（订阅 OAuth 需要 Claude Code 的 system 提示，否则 429）
  HTTPS_PROXY      可选出站代理

用法：python -I scripts/custom-api-probe.py
"""

import json
import os
import sys
import urllib.error
import urllib.request

STREAM_EVENTS = ("message_start", "content_block_start", "content_block_delta",
                 "content_block_stop", "message_delta", "message_stop")

BASE = os.environ.get("PROBE_BASE_URL", "").strip().rstrip("/")
for suffix in ("/v1/messages", "/v1"):
    if BASE.endswith(suffix):
        BASE = BASE[: -len(suffix)].rstrip("/")
KEY = os.environ.get("PROBE_API_KEY") or os.environ.get("ANTHROPIC_TEST_KEY") or ""
MODEL = os.environ.get("PROBE_MODEL", "")
if not (BASE and KEY and MODEL):
    sys.exit("需要 PROBE_BASE_URL / PROBE_API_KEY / PROBE_MODEL")


def scrub(text):
    return text.replace(KEY, "***") if KEY else text


def post(path, body, auth):
    if os.environ.get("PROBE_SYSTEM") and "messages" in body:
        body = dict(body, system=os.environ["PROBE_SYSTEM"])
    headers = {"content-type": "application/json", "anthropic-version": "2023-06-01"}
    if auth == "bearer":
        headers["authorization"] = "Bearer " + KEY
    else:
        headers["x-api-key"] = KEY
    if os.environ.get("PROBE_BETA"):
        headers["anthropic-beta"] = os.environ["PROBE_BETA"]
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(), headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:  # noqa: BLE001
        return 0, scrub(str(e))


def js(text):
    try:
        return json.loads(text)
    except ValueError:
        return {}


def err_msg(text):
    v = js(text)
    msg = (v.get("error") or {}).get("message") if isinstance(v.get("error"), dict) else None
    return scrub(msg or text)[:300]


def is_messages_response(v):
    return (isinstance(v, dict) and v.get("type") == "message" and v.get("role") == "assistant"
            and isinstance(v.get("content"), list) and isinstance(v.get("usage"), dict))


def sse_events(text):
    named = [l[6:].strip() for l in text.splitlines() if l.startswith("event:")]
    if named:
        return named
    types = []
    for l in text.splitlines():
        if l.startswith("data:"):
            v = js(l[5:].strip())
            if isinstance(v, dict) and isinstance(v.get("type"), str):
                types.append(v["type"])
    return types


def is_messages_stream(events):
    known = [e for e in events if e in STREAM_EVENTS]
    return bool(known) and known[0] == "message_start" and known[-1] == "message_stop"


def out(label, value):
    print(f"{label}: {scrub(str(value))}")


def main():
    hello = {"model": MODEL, "max_tokens": 16, "messages": [{"role": "user", "content": "Reply with OK."}]}
    styles = [os.environ["PROBE_AUTH"]] if os.environ.get("PROBE_AUTH") else ["x-api-key", "bearer"]
    auth, first = None, None
    for s in styles:
        st, text = post("/v1/messages", hello, s)
        out(f"auth[{s}]", f"HTTP {st}" + ("" if 200 <= st < 300 else " " + err_msg(text)))
        if 200 <= st < 300:
            auth, first = s, js(text)
            break
    if not auth:
        return
    if not is_messages_response(first):
        out("format", "FAIL 非流式响应不是 Anthropic Messages 形状")
        return
    out("shape", {k: first.get(k) for k in ("type", "role", "model", "stop_reason")}
        | {"id_prefix": str(first.get("id", ""))[:4], "usage": first.get("usage"),
           "content_types": [b.get("type") for b in first["content"] if isinstance(b, dict)]})

    st, text = post("/v1/messages", dict(hello, stream=True), auth)
    events = sse_events(text) if 200 <= st < 300 else []
    out("stream", f"HTTP {st} events={events[:3]}...{events[-2:]}" + ("" if 200 <= st < 300 else " " + err_msg(text)))
    out("format", "PASS" if is_messages_stream(events) else "FAIL 流式响应不是 Anthropic Messages 事件序列")

if __name__ == "__main__":
    main()
