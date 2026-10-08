#!/usr/bin/env python3
"""Writes a fake home with invented Cursor sessions, for the demo GIF and the
README examples. Every name, path and message here is made up.

    python3 demo/make-fixture.py [OUT_DIR]    # default: demo/home

Point the CLI at it with HOME=<OUT_DIR>. The IDE database is written to both
the macOS and the Linux location.
"""

import hashlib
import json
import shutil
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path


def ms(iso):
    return int(datetime.fromisoformat(iso).replace(tzinfo=timezone.utc).timestamp() * 1000)


AGENT = [
    {
        "id": "a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027",
        "title": "Add retry with backoff to the webhook sender",
        "cwd": "/Users/dana/src/billing-api",
        "model": "claude-4.5-sonnet",
        "created": "2026-10-05T13:40:12",
        "updated": "2026-10-05T14:12:47",
        "messages": [
            ("user", "Monday, Oct 5, 2026, 8:40 AM (UTC-5)",
             "Webhook deliveries fail for good on the first 503. Add retries with exponential backoff."),
            ("assistant", None,
             "I added `RetryPolicy` to `webhooks/sender.rs`: 5 attempts, backoff from 500 ms with "
             "jitter, only on 5xx responses and timeouts."),
            ("user", "Monday, Oct 5, 2026, 9:05 AM (UTC-5)", "Make the attempt count configurable."),
            ("assistant", None,
             "Done. `WEBHOOK_MAX_ATTEMPTS` (default 5) is read in `Config::from_env`, and the tests "
             "cover 1, 3 and 5 attempts."),
        ],
    },
    {
        "id": "3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63",
        "title": "Fix flaky timezone test in invoice due dates",
        "cwd": "/Users/dana/src/invoice-service",
        "model": "gpt-5",
        "created": "2026-10-02T08:03:55",
        "updated": "2026-10-02T08:31:09",
        "messages": [
            ("user", "Friday, Oct 2, 2026, 10:03 AM (UTC+2)",
             "test_due_date_end_of_month fails only on CI. Why?"),
            ("assistant", None,
             "CI runs in UTC and your laptop in Europe/Berlin. The test builds the due date with "
             "`Local::now()`, so the month boundary moves. Use a fixed `Utc` timestamp."),
            ("user", "Friday, Oct 2, 2026, 10:29 AM (UTC+2)", "Thanks, that fixed it."),
        ],
    },
    {
        "id": "6c0e4b9a-3d21-4f87-9a5c-e1b7d2f04a38",
        "title": "Backfill customer regions in a migration",
        "cwd": "/Users/dana/src/billing-api",
        "model": "claude-4.5-sonnet",
        "created": "2026-09-24T15:02:40",
        "updated": "2026-09-24T15:37:18",
        "messages": [
            ("user", "Thursday, Sep 24, 2026, 10:02 AM (UTC-5)",
             "Write a migration that fills customers.region from the billing address country."),
            ("assistant", None,
             "Added `20260924_backfill_customer_regions.sql`. It updates in batches of 5,000 rows "
             "so the table is never locked for long."),
        ],
    },
    {
        "id": "b8e27f40-91c6-4e3d-8f0a-5d6c3b2a1e97",
        "title": "Cache the Rust workspace in GitHub Actions",
        "cwd": "/Users/dana/src/platform-tools",
        "model": "gpt-5",
        "created": "2026-09-19T09:12:03",
        "updated": "2026-09-19T09:40:55",
        "messages": [
            ("user", "Saturday, Sep 19, 2026, 11:12 AM (UTC+2)", "CI spends 6 minutes compiling dependencies. Cache them."),
            ("assistant", None,
             "Added `Swatinem/rust-cache@v2` after the toolchain step. Warm runs now take about "
             "70 seconds."),
        ],
    },
]

IDE = [
    {
        "id": "e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46",
        "title": "Speed up the monthly revenue dashboard query",
        "created": "2026-10-04T16:20:00",
        "updated": "2026-10-04T16:48:31",
        "messages": [
            (1, "2026-10-04T16:21:07", "The monthly revenue panel takes 9 seconds to load. Can you speed up this query?", []),
            (2, "2026-10-04T16:22:40",
             "It scans every invoice and groups by a computed month. Add an index on "
             "`(status, issued_at)` and filter on a range so it can use it:",
             [("sql", "CREATE INDEX invoices_status_issued_at ON invoices (status, issued_at);")]),
            (1, "2026-10-04T16:48:31", "Down to 120 ms. Thanks!", []),
        ],
    },
    {
        "id": "9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864",
        "title": "Explain the migration lock error",
        "created": "2026-09-28T10:55:12",
        "updated": "2026-09-28T11:09:44",
        "messages": [
            (1, "2026-09-28T10:55:12", "What does `could not obtain lock on relation \"customers\"` mean during deploy?", []),
            (2, "2026-09-28T10:56:30",
             "Another transaction held a lock on `customers` while the migration ran `ALTER TABLE`. "
             "Set a `lock_timeout` and retry, or run it outside peak hours.", []),
        ],
    },
    {
        "id": "4a6d1c88-0f3e-4b52-96d7-c2e8a1b5f309",
        "title": "Add dark mode to the settings page",
        "created": "2026-09-22T13:30:00",
        "updated": "2026-09-22T14:05:26",
        "messages": [
            (1, "2026-09-22T13:30:00", "Add a dark mode toggle to the settings page and remember the choice.", []),
            (2, "2026-09-22T13:33:41",
             "The toggle sets `data-theme` on `<html>` and stores it in `localStorage`; "
             "`prefers-color-scheme` is the default.", []),
        ],
    },
    {
        "id": "d3c9f7e1-5b84-4a06-b2e9-7f1a6c4d8b20",
        "title": "Review the OAuth callback handler for CSRF",
        "created": "2026-09-16T08:44:19",
        "updated": "2026-09-16T09:02:57",
        "messages": [
            (1, "2026-09-16T08:44:19", "Is our OAuth callback safe against CSRF?", []),
            (2, "2026-09-16T08:46:02",
             "Not yet: the `state` parameter is generated but never checked. Compare it with the "
             "value stored in the session before exchanging the code.", []),
        ],
    },
]


def write_agent(home):
    for s in AGENT:
        cwd_hash = hashlib.md5(s["cwd"].encode()).hexdigest()
        session_dir = home / ".cursor" / "chats" / cwd_hash / s["id"]
        session_dir.mkdir(parents=True)
        meta = {
            "schemaVersion": 1,
            "createdAtMs": ms(s["created"]),
            "hasConversation": True,
            "title": s["title"],
            "updatedAtMs": ms(s["updated"]),
            "cwd": s["cwd"],
        }
        (session_dir / "meta.json").write_text(json.dumps(meta))

        store = sqlite3.connect(session_dir / "store.db")
        store.executescript(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB);"
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);"
        )
        store_meta = {"name": s["title"], "lastUsedModel": s["model"], "createdAt": ms(s["created"])}
        store.execute("INSERT INTO meta VALUES ('0', ?)", (json.dumps(store_meta).encode().hex(),))
        store.commit()
        store.close()

        project = s["cwd"].strip("/").replace("/", "-")
        transcript = home / ".cursor" / "projects" / project / "agent-transcripts" / s["id"] / f"{s['id']}.jsonl"
        transcript.parent.mkdir(parents=True)
        lines = []
        for role, stamp, text in s["messages"]:
            if role == "user":
                text = f"<timestamp>{stamp}</timestamp>\n<user_query>\n{text}\n</user_query>"
                parts = [{"type": "text", "text": text}]
            else:
                parts = [
                    {"type": "tool_use", "name": "Read", "input": {"path": "src/lib.rs"}},
                    {"type": "text", "text": text},
                ]
            lines.append(json.dumps({"role": role, "message": {"content": parts}}))
        transcript.write_text("\n".join(lines) + "\n")


def write_ide(path):
    path.parent.mkdir(parents=True)
    db = sqlite3.connect(path)
    db.executescript(
        "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);"
        "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);"
    )
    for s in IDE:
        headers = []
        for n, (kind, stamp, text, code) in enumerate(s["messages"], 1):
            bubble_id = f"{s['id'][:8]}-b{n}"
            headers.append({"bubbleId": bubble_id, "type": kind})
            bubble = {
                "bubbleId": bubble_id,
                "type": kind,
                "text": text,
                "timestamp": ms(stamp),
                "codeBlocks": [{"language": lang, "content": body} for lang, body in code],
            }
            db.execute("INSERT INTO cursorDiskKV VALUES (?, ?)", (f"bubbleId:{s['id']}:{bubble_id}", json.dumps(bubble)))
        composer = {
            "composerId": s["id"],
            "name": s["title"],
            "createdAt": ms(s["created"]),
            "lastUpdatedAt": ms(s["updated"]),
            "fullConversationHeadersOnly": headers,
        }
        db.execute("INSERT INTO cursorDiskKV VALUES (?, ?)", (f"composerData:{s['id']}", json.dumps(composer)))
    db.commit()
    db.close()


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent / "home"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    write_agent(out)
    db = out / "Library" / "Application Support" / "Cursor" / "User" / "globalStorage" / "state.vscdb"
    write_ide(db)
    linux_db = out / ".config" / "Cursor" / "User" / "globalStorage" / "state.vscdb"
    linux_db.parent.mkdir(parents=True)
    shutil.copy(db, linux_db)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
