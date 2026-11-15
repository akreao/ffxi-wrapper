"""Fails when the website's SQL (web/src/game.rs) needs a table or column
that the xiweb grants in docker/lsb/db-init.sh don't cover.

Reads need SELECT on every table named after FROM/JOIN/UPDATE/INTO/DELETE
(column-level SELECT counts: the code only reads key columns there).
Writes need DELETE, INSERT and UPDATE on the tables and columns written.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = (ROOT / "web/src/game.rs").read_text()
INIT = (ROOT / "docker/lsb/db-init.sh").read_text()


def statements():
    for lit in re.findall(r'"((?:[^"\\]|\\.)*)"', SRC, re.S):
        # A statement may start with an interpolated prefix such as `{SLOW_LIMIT} `.
        if re.match(r"\s*(?:\{\w+\}\s*)?(SELECT|UPDATE|INSERT|DELETE)\b", lit):
            yield re.sub(r"\s+", " ", lit)


def split_top(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        depth += (ch == "(") - (ch == ")")
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    return out + [cur]


def grants():
    """{(priv, table): set of columns, or None for the whole table}"""
    g = {}

    def add(priv, table, cols):
        cur = g.get((priv, table), set())
        g[(priv, table)] = None if cols is None or cur is None else cur | cols

    m = re.search(r'^\s*tables="([^"]*)"', INIT, re.M)
    for t in m.group(1).split():
        add("SELECT", t, None)
    for m in re.finditer(r"GRANT\s+([^;]+?)\s+ON\s+\\`\$db\\`\.\\`(\w+)\\`\s+TO\s+'xiweb'", INIT):
        for p in split_top(m.group(1)):
            pm = re.match(r"\s*(\w+)\s*(?:\(([^)]*)\))?\s*$", p)
            cols = {c.strip() for c in pm.group(2).split(",")} if pm.group(2) else None
            add(pm.group(1).upper(), m.group(2), cols)
    return g


def covered(g, priv, table, cols=None):
    if (priv, table) not in g:
        return False
    have = g[(priv, table)]
    return have is None or cols is None or set(cols) <= have


class WebGrants(unittest.TestCase):
    def test_every_statement_is_covered(self):
        g, seen = grants(), 0
        for s in statements():
            seen += 1
            for m in re.finditer(r"\b(?:FROM|JOIN)\s+(\w+)", s):
                self.assertTrue(covered(g, "SELECT", m.group(1)) or covered(g, "DELETE", m.group(1)), (s, m.group(0)))
            for m in re.finditer(r"\bDELETE FROM\s+(\w+)", s):
                self.assertTrue(covered(g, "DELETE", m.group(1)), s)
            dup = re.search(r"ON DUPLICATE KEY UPDATE (.*)$", s)
            body = s[: dup.start()] if dup else s
            if dup:
                tbl = re.search(r"INSERT INTO (\w+)", s).group(1)
                # MariaDB wants UPDATE on every inserted column, not only the updated ones.
                cols = re.findall(r"(\w+) = VALUES", dup.group(1)) + re.search(r"INSERT INTO \w+\s*\(([^)]*)\)", s).group(1).replace(" ", "").split(",")
                self.assertTrue(covered(g, "UPDATE", tbl, cols), (s, cols))
            for m in re.finditer(r"\bINSERT INTO\s+(\w+)\s*\(([^)]*)\)", body):
                cols = [c.strip() for c in m.group(2).split(",")]
                self.assertTrue(covered(g, "INSERT", m.group(1), cols), (s, cols))
            for m in re.finditer(r"\bUPDATE\s+(\w+)\s+SET\s+(.*?)(?:\sWHERE\b|$)", body):
                cols = re.findall(r"(\w+)\s*=", m.group(2))
                self.assertTrue(covered(g, "UPDATE", m.group(1), cols), (s, cols))
        self.assertGreater(seen, 30)  # the parser still finds the SQL

    def test_nothing_wider_than_needed(self):
        # Every non-SELECT grant must be used by some statement.
        used = {"DELETE": set(), "INSERT": set(), "UPDATE": set()}
        for s in statements():
            used["DELETE"] |= set(re.findall(r"\bDELETE FROM\s+(\w+)", s))
            used["INSERT"] |= set(re.findall(r"\bINSERT INTO\s+(\w+)", s))
            used["UPDATE"] |= set(re.findall(r"\bUPDATE\s+(\w+)\s+SET", s))
            used["UPDATE"] |= set(re.findall(r"INSERT INTO (\w+).*ON DUPLICATE", s))
        for (priv, table) in grants():
            if priv != "SELECT":
                self.assertIn(table, used[priv], f"{priv} on {table} is not used by the web code")

    def test_item_database_is_read_only(self):
        # The item pages show static game data to every signed-in player: SELECT and nothing else.
        g = grants()
        for t in ("item_basic", "item_equipment", "item_weapon", "item_mods"):
            self.assertTrue(covered(g, "SELECT", t), t)
            for priv in ("INSERT", "UPDATE", "DELETE"):
                self.assertNotIn((priv, t), g, f"{priv} on {t}")

    def test_audit_logs_are_read_only(self):
        # The audit logs page only reads LSB's audit tables; the site never edits or prunes them.
        g = grants()
        for t in ("audit_chat", "audit_gm", "audit_trade", "audit_bazaar", "audit_dbox"):
            self.assertTrue(covered(g, "SELECT", t), t)
            for priv in ("INSERT", "UPDATE", "DELETE"):
                self.assertNotIn((priv, t), g, f"{priv} on {t}")
            self.assertIn(t, SRC, f"the web code never reads {t}")


if __name__ == "__main__":
    unittest.main()
