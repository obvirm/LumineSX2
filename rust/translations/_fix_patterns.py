#!/usr/bin/env python3
"""
Fix 'CONST + N' in match pattern arms.

For each match arm like:
    FOO + 2 => something

Convert to either:
    n if n == FOO + 2 => something
(if there is a `x if` already)

or just keep `FOO + 2 => something` if the file is too complex to parse.

Strategy: only fix if pattern is bare, leave `x if x == ...` alone.
"""
import re
from pathlib import Path

root = Path("E:/project/pcsx2/rust/translations")
fix_count = 0
file_count = 0

# Regex: capture optional leading pattern ident, then CONST + NUM, then =>
# Group 1: optional binding (e.g. "x if x == " or "x if ")
# Group 2: CONST name
# Group 3: + NUM
# Group 4: =>
# We need to handle: `FOO + 2 => ...` and convert to `n if n == FOO + 2 => ...`
# But match arms need a binding like `x if x == ...` OR pure pattern.

# Actually the simplest fix: change `FOO + 2 =>` to `FOO2 =>` (i.e. pre-compute
# the constant value at the top of the file or in the match as a guard).

# Better: convert `FOO + 2 => ...` to `_ if _ == FOO + 2 => ...`
# But the variable name matters. Let's just convert to `x if x == FOO + 2 => ...`
# where x is a generic name.

# Strategy 2: simplest - just convert `IDENT + NUM =>` to `IDENT_PLUS_NUM =>` but
# we can't easily insert definitions. Instead, use match guards:
#   FOO + 2 => ... -> x if x == FOO + 2 => ...

# But we need to know what `x` is bound to. Look for the match subject: `match X {`
# Parse backwards to find the binding name.

def fix_match_patterns(text: str) -> str:
    """Fix `IDENT + NUM =>` in match patterns to use guards."""
    lines = text.splitlines(keepends=True)
    out = []
    i = 0
    while i < len(lines):
        line = lines[i]
        # Detect "match X {" line and capture the subject X
        m = re.match(r'^(\s*)match\s+([A-Za-z_][A-Za-z0-9_:\.]*)\s*\{', line)
        if m:
            indent = m.group(1)
            subject = m.group(2)
            out.append(line)
            i += 1
            # Process arms until matching closing brace at same indent
            depth = 1
            while i < len(lines) and depth > 0:
                arm_line = lines[i]
                # Count braces
                opens = arm_line.count('{')
                closes = arm_line.count('}')
                # Check if this is the closing brace
                if re.match(r'^\s*\}', arm_line):
                    depth -= 1
                    out.append(arm_line)
                    i += 1
                    continue
                # Find pattern: leading whitespace, then ident or pattern
                m2 = re.match(r'^(\s*)(.+?)\s*=>', arm_line)
                if m2 and depth == 1:
                    arm_indent = m2.group(1)
                    pattern = m2.group(2).strip()
                    # Skip if it's a guard already (contains ' if ')
                    if ' if ' not in pattern:
                        # Check if pattern has `IDENT + NUM` arithmetic
                        m3 = re.match(r'^([A-Z_][A-Z0-9_]*)\s*([+\-*/])\s*(\d+)$', pattern)
                        if m3:
                            ident = m3.group(1)
                            op = m3.group(2)
                            num = m3.group(3)
                            # Convert to guard
                            new_pattern = f'{subject} if {subject} {op} {num}'
                            new_line = arm_line.replace(f'{pattern} =>', f'{new_pattern} =>', 1)
                            out.append(new_line)
                            i += 1
                            depth += opens - closes
                            continue
                    # Check for `IDENT as TYPE` (also a pattern)
                    m4 = re.match(r'^([A-Z_][A-Z0-9_]*)\s+as\s+(\w+)$', pattern)
                    if m4:
                        ident = m4.group(1)
                        ty = m4.group(2)
                        # Convert to guard
                        new_pattern = f'{subject} if {subject} == {ident} as {ty}'
                        new_line = arm_line.replace(f'{pattern} =>', f'{new_pattern} =>', 1)
                        out.append(new_line)
                        i += 1
                        depth += opens - closes
                        continue
                out.append(arm_line)
                depth += opens - closes
                i += 1
            continue
        out.append(line)
        i += 1
    return ''.join(out)


for f in root.rglob("*.rs"):
    if "target" in f.parts:
        continue
    text = f.read_text(encoding="utf-8")
    if " + " not in text and " as " not in text:
        continue
    if "match " not in text:
        continue
    new_text = fix_match_patterns(text)
    if new_text != text:
        # Count changes
        n_changes = sum(1 for a, b in zip(text.splitlines(), new_text.splitlines()) if a != b)
        f.write_text(new_text, encoding="utf-8")
        rel = str(f).replace("E:/project/pcsx2/rust/translations/", "")
        print(f"  {rel}: {n_changes} lines modified")
        fix_count += 1
        file_count += 1

print(f"Modified {file_count} files, {fix_count} total")
