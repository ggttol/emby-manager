#!/usr/bin/env python3
"""Remove confirmed-invalid 115 shares from the catalog database."""

import argparse
import json
import sqlite3
import time
from pathlib import Path

from validate_catalog_115_links import parse_115_url


ROOT = Path(__file__).resolve().parents[1]


def backup_database(source, destination):
    with sqlite3.connect(source) as src, sqlite3.connect(destination) as dst:
        src.backup(dst)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", default=str(ROOT / "catalog_115.db"))
    parser.add_argument("--validation-db", default=str(ROOT / "catalog_115_validation.db"))
    parser.add_argument("--backup", default="", help="backup path; defaults to a timestamped file")
    args = parser.parse_args(argv)

    db_path = Path(args.db)
    validation_path = Path(args.validation_db)
    if not db_path.exists() or not validation_path.exists():
        parser.error("catalog or validation database does not exist")

    stamp = time.strftime("%Y%m%d-%H%M%S")
    backup_path = Path(args.backup) if args.backup else db_path.with_name(f"{db_path.name}.bak-invalid-{stamp}")
    backup_database(db_path, backup_path)

    with sqlite3.connect(validation_path) as validation:
        invalid = set(
            validation.execute(
                "SELECT share_code, receive_code FROM share_checks WHERE status='invalid'"
            )
        )

    removed_pairs = set()
    removed_rows = 0
    with sqlite3.connect(db_path) as catalog:
        rows = catalog.execute(
            "SELECT rowid, link FROM catalog WHERE link_type='share115'"
        ).fetchall()
        rowids = []
        for rowid, link in rows:
            pair = parse_115_url(link)
            if pair in invalid:
                rowids.append((rowid,))
                removed_pairs.add(pair)

        catalog.execute("BEGIN IMMEDIATE")
        catalog.executemany("DELETE FROM catalog WHERE rowid=?", rowids)
        removed_rows = catalog.total_changes
        catalog.commit()
        integrity = catalog.execute("PRAGMA integrity_check").fetchone()[0]
        remaining = catalog.execute("SELECT COUNT(*) FROM catalog").fetchone()[0]
        remaining_shares = catalog.execute(
            "SELECT COUNT(*) FROM catalog WHERE link_type='share115'"
        ).fetchone()[0]

    print(
        json.dumps(
            {
                "backup": str(backup_path),
                "confirmed_invalid_pairs": len(invalid),
                "matched_invalid_pairs": len(removed_pairs),
                "already_absent_pairs": len(invalid - removed_pairs),
                "removed_rows": removed_rows,
                "remaining_rows": remaining,
                "remaining_share115_rows": remaining_shares,
                "integrity_check": integrity,
            },
            ensure_ascii=False,
        )
    )
    return 0 if integrity == "ok" else 1


if __name__ == "__main__":
    raise SystemExit(main())
