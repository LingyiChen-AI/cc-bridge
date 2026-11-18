"""探测一个 Anthropic 兼容接口是否像真 Claude（与 custom_api/verify.rs 的请求形状一致）。

环境变量（key 只从环境变量读，不打印、不写文件）：
  PROBE_BASE_URL   如 https://api.anthropic.com（可带 /v1 或 /v1/messages，会去掉）
  PROBE_API_KEY    接口 key；未设时读 ANTHROPIC_TEST_KEY
  PROBE_MODEL      如 claude-haiku-4-5
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

FIXED_TEXT = (
    "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳过了懒狗。 "
    "0123456789 !@#$%^&*() éè \U0001F600"
)
QUESTION = "A train leaves at 9:47 and arrives at 13:12. How many minutes is the trip? Reason carefully."
FALLBACK_QUESTION = "Is 391 prime? Check divisors step by step."

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


def thinking_body(messages):
    legacy = any(MODEL.startswith(p) for p in ("claude-haiku-4-5", "claude-sonnet-4-", "claude-opus-4-"))
    if legacy:
        return {"model": MODEL, "max_tokens": 2048, "messages": messages,
                "thinking": {"type": "enabled", "budget_tokens": 1024}}
    return {"model": MODEL, "max_tokens": 4096, "messages": messages,
            "thinking": {"type": "adaptive", "display": "omitted"}, "output_config": {"effort": "high"}}


def tamper(sig):
    if not sig:
        return "A"
    c = list(sig)
    i = max(len(c) - 8, 0)
    c[i] = "B" if c[i] == "A" else "A"
    return "".join(c)


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
    out("shape", {k: first.get(k) for k in ("type", "role", "model", "stop_reason")}
        | {"id_prefix": str(first.get("id", ""))[:4], "usage": first.get("usage"),
           "content_types": [b.get("type") for b in first.get("content", [])]})

    st, text = post("/v1/messages", dict(hello, stream=True), auth)
    events = [l[6:].strip() for l in text.splitlines() if l.startswith("event:")]
    out("stream", f"HTTP {st} events={events[:3]}...{events[-2:]}")

    msgs = [{"role": "user", "content": FIXED_TEXT}]
    st, text = post("/v1/messages/count_tokens", {"model": MODEL, "messages": msgs}, auth)
    out("count_tokens", f"HTTP {st} " + (str(js(text).get("input_tokens")) if st == 200 else err_msg(text)))
    st, text = post("/v1/messages", {"model": MODEL, "max_tokens": 16, "messages": msgs}, auth)
    out("usage_input", f"HTTP {st} " + (str((js(text).get("usage") or {}).get("input_tokens")) if st == 200 else err_msg(text)))

    found = None
    for q in (QUESTION, FALLBACK_QUESTION):
        st, text = post("/v1/messages", thinking_body([{"role": "user", "content": q}]), auth)
        v = js(text)
        blocks = v.get("content", []) if st == 200 else []
        out("thinking", f"HTTP {st} blocks=" + str([
            {"type": b.get("type"), "sig_len": len(b.get("signature") or ""), "data_len": len(b.get("data") or ""),
             "thinking_len": len(b.get("thinking") or "")} for b in blocks
        ]) + ("" if st == 200 else " " + err_msg(text)))
        blk = next((b for b in blocks if b.get("type") in ("thinking", "redacted_thinking")
                    and (b.get("signature") or b.get("data"))), None)
        if blk:
            found = (q, blk, [b for b in blocks if b.get("type") == "text"])
            break
    if not found:
        out("signature", "无签名块")
        return
    q, blk, texts = found
    field = "signature" if blk.get("signature") else "data"
    out("signed_block", f"type={blk['type']} field={field}")

    def replay(b):
        return thinking_body([{"role": "user", "content": q},
                              {"role": "assistant", "content": [b] + texts},
                              {"role": "user", "content": "Now add 1."}])

    def show(label, b):
        st, text = post("/v1/messages", replay(b), auth)
        ok = 200 <= st < 300
        detail = f"input_tokens={(js(text).get('usage') or {}).get('input_tokens')}" if ok else err_msg(text)
        out(label, f"HTTP {st} {detail}")

    show("replay_original", blk)
    bad = dict(blk)
    bad[field] = tamper(blk[field])
    show("replay_tampered(len-8)", bad)
    # 对照：改签名中段，区分「签名尾部不参与校验」和「签名根本没被校验」
    mid = list(blk[field])
    i = len(mid) // 2
    mid[i] = "B" if mid[i] == "A" else "A"
    show("replay_tampered(mid)", dict(blk, **{field: "".join(mid)}))


if __name__ == "__main__":
    main()
