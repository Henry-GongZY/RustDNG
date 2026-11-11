#!/usr/bin/env python3
"""Transcribe dng_tag_values.h and dng_tag_codes.h (DNG SDK 1.7.1) into Rust
const modules for crates/dng-core.

Usage: python3 -I tools/transcribe_tags.py <header> <out.rs>

Evaluates simple C enum expressions (integer literals in dec/hex/oct, prior
enum names, + - * / << >> parens) so every constant keeps its exact C++ value.
Comments are preserved as Rust comments so the output stays traceable.
"""

import re
import sys

UINT32_MASK = 0xFFFFFFFF


class EnumTranscriber:
    def __init__(self):
        self.values = {}
        self.out = []

    def eval_expr(self, expr):
        # Tokenize: names, numbers, operators, parens.
        token_re = re.compile(
            r"\s*(?:(?P<num>0[xX][0-9a-fA-F]+|\d+)|(?P<name>[A-Za-z_]\w*)|(?P<op>[-+*/()<>]))"
        )
        tokens = []
        pos = 0
        while pos < len(expr):
            m = token_re.match(expr, pos)
            if not m:
                if expr[pos:].strip() == "":
                    break
                raise ValueError(f"bad token at {expr[pos:]!r} in {expr!r}")
            pos = m.end()
            if m.group("num") is not None:
                tokens.append(("num", int(m.group("num"), 0)))
            elif m.group("name") is not None:
                tokens.append(("name", m.group("name")))
            else:
                op = m.group("op")
                # '<<' / '>>' come through as two '<'/'>' chars.
                if op in "<>":
                    if pos < len(expr) and expr[pos] == op:
                        pos += 1
                        op *= 2
                tokens.append(("op", op))

        # Recursive descent parser: term (('+'|'-') term)*
        idx = [0]

        def peek():
            return tokens[idx[0]] if idx[0] < len(tokens) else None

        def take():
            t = tokens[idx[0]]
            idx[0] += 1
            return t

        def parse_shift():
            left = parse_addsub()
            while True:
                t = peek()
                if t and t[0] == "op" and t[1] in ("<<", ">>"):
                    take()
                    right = parse_addsub()
                    left = left << right if t[1] == "<<" else left >> right
                else:
                    return left

        def parse_addsub():
            left = parse_muldiv()
            while True:
                t = peek()
                if t and t[0] == "op" and t[1] in "+-":
                    take()
                    right = parse_muldiv()
                    left = left + right if t[1] == "+" else left - right
                else:
                    return left

        def parse_muldiv():
            left = parse_unary()
            while True:
                t = peek()
                if t and t[0] == "op" and t[1] in "*/":
                    take()
                    right = parse_unary()
                    left = left * right if t[1] == "*" else left // right
                else:
                    return left

        def parse_unary():
            t = peek()
            if t and t[0] == "op" and t[1] == "-":
                take()
                return -parse_unary()
            if t and t[0] == "op" and t[1] == "+":
                take()
                return parse_unary()
            if t and t[0] == "op" and t[1] == "(":
                take()
                v = parse_shift()
                t = take()
                assert t == ("op", ")"), f"expected ) in {expr!r}"
                return v
            t = take()
            if t[0] == "num":
                return t[1]
            if t[0] == "name":
                if t[1] not in self.values:
                    raise KeyError(f"unknown name {t[1]!r} in {expr!r}")
                return self.values[t[1]]
            raise ValueError(f"unexpected token {t} in {expr!r}")

        value = parse_shift()
        if idx[0] != len(tokens):
            raise ValueError(f"trailing tokens in {expr!r}")
        return value

    def transcribe(self, header_text, out_path):
        lines = header_text.splitlines()

        # Collect the whole set of names first (for expression eval order we
        # evaluate lazily with memoized names).
        pending = []  # (name, expr_or_None, comment)
        skip_depth = []  # conditional-compilation skip levels
        block_id = -1  # increments per enum block

        in_enum = False
        enum_name = None
        brace_depth = 0
        saw_enum_kw = False
        pending_enum_name = None

        for raw in lines:
            line = raw.strip()

            if not in_enum:
                if saw_enum_kw:
                    # Expect the opening brace (possibly with a name before it).
                    if line.startswith("{"):
                        in_enum = True
                        enum_name = pending_enum_name
                        saw_enum_kw = False
                        brace_depth = line.count("{") - line.count("}")
                        block_id += 1
                        continue
                    m = re.match(r"^([A-Za-z_]\w*)\s*\{$", line)
                    if m:
                        in_enum = True
                        enum_name = m.group(1)
                        saw_enum_kw = False
                        brace_depth = 1
                        block_id += 1
                        continue
                    # 'enum Name' followed by '{' later.
                    m = re.match(r"^(?:enum\s+)?([A-Za-z_]\w*)$", line)
                    if m and not line.startswith("enum"):
                        pending_enum_name = m.group(1)
                        continue
                    if not line:
                        continue
                    saw_enum_kw = False
                    continue
                if re.match(r"^enum(\s|$)", line):
                    saw_enum_kw = True
                    pending_enum_name = None
                    m = re.match(r"^enum\s+([A-Za-z_]\w*)$", line)
                    if m:
                        pending_enum_name = m.group(1)
                    if "{" in line:
                        in_enum = True
                        saw_enum_kw = False
                        enum_name = pending_enum_name
                        brace_depth = line.count("{") - line.count("}")
                    continue
                continue

            # Inside an enum.
            brace_depth += line.count("{") - line.count("}")
            if brace_depth <= 0 and "}" in line:
                in_enum = False
                enum_name = None
                continue

            if line.startswith("#"):
                # Single-level conditional support for the SDK's feature
                # flags (all default per dng_flags.h).
                m_if = re.match(r"#\s*if\s+(\w+)", line)
                if m_if:
                    flag = m_if.group(1)
                    known = {
                        "qDNGSupportVC5": True,
                        "qDNGExperimental": True,
                        "qDNGBigImage": True,
                        "qDNGUseXMP": True,
                    }
                    if flag not in known:
                        raise ValueError(f"unknown conditional flag {flag!r}")
                    skip_depth.append(0 if known[flag] else 1)
                    continue
                m_elif = re.match(r"#\s*elif\s+(\w+)", line)
                if m_elif:
                    raise ValueError("unsupported #elif: " + line)
                if re.match(r"#\s*else", line):
                    if skip_depth:
                        skip_depth[-1] = 1 - skip_depth[-1]
                    continue
                if re.match(r"#\s*endif", line):
                    if skip_depth:
                        skip_depth.pop()
                    continue
                raise ValueError("unsupported directive in enum: " + line)

            if any(skip_depth):
                continue

            if not line:
                continue

            # Split trailing comment.
            comment = None
            if "//" in line:
                code, comment = line.split("//", 1)
                code = code.strip()
                comment = comment.strip()
            else:
                code = line.rstrip()

            if not code or code == ",":
                continue

            code = code.rstrip(",").strip()
            if not code:
                continue

            if "=" in code:
                name, expr = code.split("=", 1)
                pending.append((name.strip(), expr.strip(), comment, block_id))
            else:
                pending.append((code, None, comment, block_id))

        # Resolve in declaration order; bare names follow C's sequential rule
        # (previous + 1 within the same enum block, starting at 0).
        last_value_by_block = {}
        for name, expr, comment, block_id in pending:
            if expr is None:
                value = last_value_by_block.get(block_id, -1) + 1
            else:
                value = self.eval_expr(expr)
            last_value_by_block[block_id] = value
            self.values[name] = value
            self.out.append((name, value, comment))

        with open(out_path, "w") as f:
            f.write("// GENERATED by tools/transcribe_tags.py from %s — do not edit by hand;\n" % sys.argv[1])
            f.write("// regenerate with: python3 -I tools/transcribe_tags.py <header> <out>\n\n")
            f.write("use crate::dng_types::uint32;\n\n")
            for name, value, comment in self.out:
                cpp_val = value
                rust_val = value & UINT32_MASK
                if cpp_val < 0:
                    f.write(f"// C++ value {cpp_val} wraps to {rust_val} in uint32\n")
                if comment:
                    f.write(f"/// {comment}\n")
                f.write(f"pub const {name}: uint32 = {rust_val};\n")
                f.write("\n" if False else "")

        return len(self.out)


def main():
    header_path, out_path = sys.argv[1], sys.argv[2]
    with open(header_path) as f:
        text = f.read()
    t = EnumTranscriber()
    count = t.transcribe(text, out_path)
    print(f"{header_path}: emitted {count} constants -> {out_path}")


if __name__ == "__main__":
    main()
