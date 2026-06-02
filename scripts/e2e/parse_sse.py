#!/usr/bin/env python3
"""Parse SSE transcript file from daemon, output distinct event types and text length.

Usage:
    python3 parse_sse.py <transcript-file>
Output (one per line):
    distinct_event_types (comma-separated)
    TEXT_LEN=<N>
    PREVIEW=<first 200 chars of text>
"""
import json
import sys


def main():
    if len(sys.argv) < 2:
        print("usage: parse_sse.py <transcript>", file=sys.stderr)
        sys.exit(1)
    path = sys.argv[1]
    distinct = set()
    text_parts = []
    usage_total_in = 0
    usage_total_out = 0
    error_events = []
    model = None
    stop_reason = None
    parsed = 0
    failed = 0
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line.startswith("data: "):
                continue
            payload = line[6:].strip()
            if not payload:
                continue
            try:
                ev = json.loads(payload)
            except json.JSONDecodeError:
                failed += 1
                continue
            parsed += 1
            t = ev.get("type")
            if t:
                distinct.add(t)
            if t == "text_delta":
                text_parts.append(ev.get("text", ""))
            elif t == "message_start":
                model = ev.get("model")
            elif t == "usage":
                usage_total_in = ev.get("input_tokens", 0)
                usage_total_out = ev.get("output_tokens", 0)
            elif t == "message_delta":
                stop_reason = ev.get("stop_reason")
            elif t == "error":
                error_events.append(ev)
    text = "".join(text_parts)
    print(",".join(sorted(distinct)))
    print(f"TEXT_LEN={len(text)}")
    preview = text[:200].replace("\n", "\\n")
    print(f"PREVIEW={preview}")
    print(f"MODEL={model}")
    print(f"STOP_REASON={stop_reason}")
    print(f"USAGE_IN={usage_total_in}")
    print(f"USAGE_OUT={usage_total_out}")
    print(f"PARSED={parsed}")
    print(f"FAILED={failed}")
    if error_events:
        print(f"ERRORS={json.dumps(error_events, ensure_ascii=False)}")


if __name__ == "__main__":
    main()
