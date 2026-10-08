#!/usr/bin/env python3
"""Generate synthetic Cursor session stores for benchmarking cursor-session.

    python3 bench/gen.py OUT_DIR --sessions 300 --messages 200 --size 1500

Writes, under OUT_DIR:
  agent/.cursor/chats/<hash>/<id>/meta.json                       (Agent CLI metadata)
  agent/.cursor/projects/bench/agent-transcripts/<id>/<id>.jsonl   (Agent CLI transcripts)
  ide/state.vscdb                                                  (IDE store)

Session IDs are deterministic (uuid5), so `show` benchmarks can name one.
Prints the ID of the most recently updated session of each store.
"""

import argparse
import json
import os
import sqlite3
import uuid

NS = uuid.UUID("6f9a6c1e-6f5b-4d3c-9a1b-1d2e3f405060")
BASE_MS = 1_780_000_000_000


def sid(store: str, i: int) -> str:
    return str(uuid.uuid5(NS, f"{store}-{i}"))


def text(i: int, j: int, size: int) -> str:
    head = f"message {j} of session {i}. "
    return head + "lorem ipsum dolor sit amet " * (size // 27 + 1)


def agent(out: str, sessions: int, messages: int, size: int) -> str:
    base = os.path.join(out, "agent", ".cursor", "projects", "bench", "agent-transcripts")
    for i in range(sessions):
        s = sid("agent", i)
        d = os.path.join(base, s)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, f"{s}.jsonl"), "w") as f:
            for j in range(messages):
                role = "user" if j % 2 == 0 else "assistant"
                line = {"role": role, "message": {"content": [{"type": "text", "text": text(i, j, size)[:size]}]}}
                f.write(json.dumps(line) + "\n")
        chat = os.path.join(out, "agent", ".cursor", "chats", "0" * 32, s)
        os.makedirs(chat, exist_ok=True)
        meta = {
            "schemaVersion": 1,
            "createdAtMs": BASE_MS + i * 60_000,
            "hasConversation": True,
            "title": f"bench session {i}",
            "updatedAtMs": BASE_MS + i * 60_000 + 30_000,
            "cwd": "/Users/bench",
        }
        with open(os.path.join(chat, "meta.json"), "w") as f:
            json.dump(meta, f)
    return sid("agent", sessions - 1)


def ide(out: str, sessions: int, messages: int, size: int) -> str:
    d = os.path.join(out, "ide")
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, "state.vscdb")
    if os.path.exists(path):
        os.remove(path)
    db = sqlite3.connect(path)
    db.executescript(
        "PRAGMA journal_mode=wal;"
        "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);"
        "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);"
    )
    rows = []
    for i in range(sessions):
        c = sid("ide", i)
        headers = []
        for j in range(messages):
            b = f"b{j:05d}"
            kind = 1 if j % 2 == 0 else 2
            headers.append({"bubbleId": b, "type": kind})
            rows.append((f"bubbleId:{c}:{b}", json.dumps({"bubbleId": b, "type": kind, "text": text(i, j, size)[:size]})))
        composer = {
            "composerId": c,
            "name": f"bench session {i}",
            "createdAt": BASE_MS + i * 60_000,
            "lastUpdatedAt": BASE_MS + i * 60_000 + 30_000,
            "fullConversationHeadersOnly": headers,
        }
        rows.append((f"composerData:{c}", json.dumps(composer)))
        if len(rows) > 20_000:
            db.executemany("INSERT INTO cursorDiskKV VALUES (?, ?)", rows)
            rows.clear()
    db.executemany("INSERT INTO cursorDiskKV VALUES (?, ?)", rows)
    db.commit()
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()
    return sid("ide", sessions - 1)


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("out")
    p.add_argument("--sessions", type=int, default=300)
    p.add_argument("--messages", type=int, default=200)
    p.add_argument("--size", type=int, default=1500, help="bytes of text per message")
    a = p.parse_args()
    print(f"agent {agent(a.out, a.sessions, a.messages, a.size)}")
    print(f"ide {ide(a.out, a.sessions, a.messages, a.size)}")


if __name__ == "__main__":
    main()
