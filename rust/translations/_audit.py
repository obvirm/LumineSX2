#!/usr/bin/env python3
"""Parse cargo check log and group errors by code + sample messages."""
import re
from collections import defaultdict

log = open("E:/project/pcsx2/rust/translations/_audit.log", encoding="utf-8").read()
errors = re.findall(r'error\[(E\d+)\]:\s*(.+?)(?=\n)', log)
by_code = defaultdict(list)
for code, msg in errors:
    by_code[code].append(msg.strip())

print(f"Total errors: {len(errors)}")
print()
for code, msgs in sorted(by_code.items(), key=lambda x: -len(x[1])):
    print(f"[{code}] {len(msgs)} occurrences")
    # Print unique messages
    seen = set()
    for m in msgs:
        if m not in seen:
            seen.add(m)
            # Truncate
            print(f"  - {m[:130]}")
    print()
