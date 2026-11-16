#!/usr/bin/env python3
"""A reference reading of the fabric-desktop/1 contract, written apart from the Rust and
TypeScript sides. It follows contracts/desktop/v1/README.md and uses the Python standard
library only.

It has two uses. It computed the expectations stored in the fixtures, and `bless` computes
them again when a fixture is added. And it is a third side for the contract suite: its
report and its verdicts on the mutants must equal those of the other two.

usage: reference.py report [contract dir]           print the report of README section 13
       reference.py judge <mutants.jsonl> [dir]     print one verdict per mutant
       reference.py bless [contract dir]            check the fixtures and fill in expectations

`bless` writes the canonical text and digests of accepted fixtures. For a refused fixture it
changes nothing: it only checks that the layer and error written by hand are what it computes.
"""

import hashlib
import json
import os
import re
import sys

PROTOCOL = "fabric-desktop/1"
MAX_SAFE_INTEGER = 9007199254740991
MAX_DEPTH = 32
FRAME_ENTRIES = {
    "owner-channel.schema.json#/$defs/owner_request",
    "owner-channel.schema.json#/$defs/owner_response",
    "owner-channel.schema.json#/$defs/owner_notice",
    "agent-channel.schema.json#/$defs/agent_request",
    "agent-channel.schema.json#/$defs/agent_response",
}
OWNER_REQUEST = "owner-channel.schema.json#/$defs/owner_request"
AGENT_REQUEST = "agent-channel.schema.json#/$defs/agent_request"


class Refused(Exception):
    """The text is not strict JSON (README section 2)."""


# ---- Strict JSON (README section 2) ----


def _members(pairs):
    members = {}
    for name, value in pairs:
        if name in members:
            raise Refused("duplicate member " + name)
        members[name] = value
    return members


def _integer(token):
    if not re.fullmatch(r"-?(0|[1-9][0-9]*)", token) or token == "-0":
        raise Refused("number form " + token)
    number = int(token)
    if abs(number) > MAX_SAFE_INTEGER:
        raise Refused("number outside the safe range")
    return number


def _not_an_integer(token):
    raise Refused("number form " + token)


def _check(value, depth=1):
    """Refuses lone surrogates and containers nested deeper than MAX_DEPTH."""
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError:
            raise Refused("lone surrogate")
    elif isinstance(value, (dict, list)):
        if depth > MAX_DEPTH:
            raise Refused("nested deeper than 32")
        if isinstance(value, dict):
            for name, member in value.items():
                _check(name)
                _check(member, depth + 1)
        else:
            for item in value:
                _check(item, depth + 1)


def parse_strict(data):
    """Bytes to a JSON value, or Refused."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise Refused("not UTF-8")
    if text.startswith("﻿"):
        raise Refused("byte-order mark")
    try:
        value = json.loads(
            text,
            object_pairs_hook=_members,
            parse_int=_integer,
            parse_float=_not_an_integer,
            parse_constant=_not_an_integer,
        )
    except RecursionError:
        raise Refused("nested deeper than 32")
    except ValueError as error:
        raise Refused(str(error))
    _check(value)
    return value


# ---- Canonical form and digests (README sections 3 and 4) ----

_SHORT_ESCAPES = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\t": "\\t", "\n": "\\n", "\f": "\\f", "\r": "\\r"}


def _canonical_string(text):
    out = ['"']
    for character in text:
        if character in _SHORT_ESCAPES:
            out.append(_SHORT_ESCAPES[character])
        elif ord(character) < 0x20:
            out.append("\\u%04x" % ord(character))
        else:
            out.append(character)
    out.append('"')
    return "".join(out)


def canonical(value):
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return _canonical_string(value)
    if isinstance(value, list):
        return "[" + ",".join(canonical(item) for item in value) + "]"
    if isinstance(value, dict):
        # Python orders strings by code point, which is the order the contract asks for.
        return "{" + ",".join(_canonical_string(name) + ":" + canonical(value[name]) for name in sorted(value)) + "}"
    raise TypeError("not a JSON value: %r" % type(value))


def sha256_hex(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def digest(domain, value):
    return "sha256:" + sha256_hex(PROTOCOL + "\n" + domain + "\n" + canonical(value))


# ---- The contract: schemas and data files ----


def read_json(path):
    with open(path, encoding="utf-8") as file:
        return json.load(file)


def write_json(path, value):
    with open(path, "w", encoding="utf-8", newline="\n") as file:
        json.dump(value, file, indent=2, ensure_ascii=True)
        file.write("\n")


class Contract:
    def __init__(self, root):
        self.root = root
        self.fixtures = os.path.join(root, "fixtures")
        schemas = os.path.join(root, "schemas")
        self.schemas = {
            name: read_json(os.path.join(schemas, name))
            for name in sorted(os.listdir(schemas))
            if name.endswith(".schema.json")
        }
        methods = read_json(os.path.join(root, "data", "owner-methods.json"))["methods"]
        tools = read_json(os.path.join(root, "data", "tool-catalog.json"))["tools"]
        lifecycle = read_json(os.path.join(root, "data", "request-lifecycle.json"))
        self.owner_methods = {m["method"] for m in methods}
        self.commands = {m["method"] for m in methods if m["command"]}
        self.tools = {t["name"] for t in tools}
        self.actions = {t["name"] for t in tools if t["creates_request"]}
        self.initial_state = lifecycle["initial"]
        self.transitions = {(t["from"], t["to"]) for t in lifecycle["transitions"]}
        self.max_frame_bytes = read_json(os.path.join(root, "data", "limits.json"))["limits"]["max_frame_bytes"]

    # The schema subset of README section 6.

    def _resolve(self, file, reference):
        target, fragment = (file, reference[1:]) if reference.startswith("#") else reference.split("#", 1)
        node = self.schemas[target]
        for part in fragment.split("/")[1:]:
            node = node[part]
        return target, node

    def valid(self, reference, value):
        file, schema = self._resolve("", reference)
        return self._valid(file, schema, value)

    def _valid(self, file, schema, value):
        if "$ref" in schema:
            target, node = self._resolve(file, schema["$ref"])
            return self._valid(target, node, value)
        is_integer = isinstance(value, int) and not isinstance(value, bool)
        if "type" in schema:
            of_type = {
                "object": isinstance(value, dict),
                "array": isinstance(value, list),
                "string": isinstance(value, str),
                "integer": is_integer,
                "boolean": isinstance(value, bool),
                "null": value is None,
            }
            if not of_type[schema["type"]]:
                return False
        if "const" in schema and canonical(schema["const"]) != canonical(value):
            return False
        if "enum" in schema and canonical(value) not in [canonical(item) for item in schema["enum"]]:
            return False
        if isinstance(value, str):
            # len() counts code points.
            if len(value) < schema.get("minLength", 0) or len(value) > schema.get("maxLength", len(value)):
                return False
            # Every pattern is anchored with ^ and $; fullmatch states the same without them.
            if "pattern" in schema and not re.fullmatch(schema["pattern"][1:-1], value, flags=re.ASCII):
                return False
        if is_integer:
            if value < schema.get("minimum", value) or value > schema.get("maximum", value):
                return False
        if isinstance(value, list):
            if len(value) < schema.get("minItems", 0) or len(value) > schema.get("maxItems", len(value)):
                return False
            if schema.get("uniqueItems") and len({canonical(item) for item in value}) != len(value):
                return False
            if "items" in schema and not all(self._valid(file, schema["items"], item) for item in value):
                return False
        if isinstance(value, dict) and "properties" in schema:
            # Every object schema requires all its properties and allows no others.
            properties = schema["properties"]
            if set(value) != set(properties):
                return False
            if not all(self._valid(file, properties[name], value[name]) for name in properties):
                return False
        if "oneOf" in schema and sum(1 for branch in schema["oneOf"] if self._valid(file, branch, value)) != 1:
            return False
        return True

    # The layers of refusal (README section 5).

    def judge(self, entry, data):
        """("accept", value) or ("reject", layer, error)."""
        is_frame = entry in FRAME_ENTRIES
        if is_frame and (len(data) > self.max_frame_bytes or b"\n" in data):
            return ("reject", "frame", "malformed_frame")
        try:
            value = parse_strict(data)
        except Refused:
            return ("reject", "frame", "malformed_frame")
        if not isinstance(value, dict):
            return ("reject", "frame", "malformed_frame")
        if is_frame and value.get("protocol") != PROTOCOL:
            return ("reject", "protocol", "unsupported_protocol")
        if not self.valid(entry, value):
            names = self.owner_methods if entry == OWNER_REQUEST else self.tools if entry == AGENT_REQUEST else None
            method = value.get("method")
            known = isinstance(method, str) and names is not None and method in names
            return ("reject", "schema", "unknown_method" if names is not None and not known else "invalid_params")
        return ("accept", value)

    def frame_digests(self, entry, value):
        """The command digest and the action digest of an accepted frame, or None."""
        command = action = None
        if entry == OWNER_REQUEST and value["method"] in self.commands:
            command = digest("command", {"method": value["method"], "params": value["params"]})
        if entry == AGENT_REQUEST and value["method"] in self.actions:
            action = digest("action", {"tool": value["method"], "arguments": value["params"]})
        return command, action

    # The request lifecycle (README section 9).

    def first_invalid(self, states):
        """The index of the first state that breaks the lifecycle, or None."""
        if not states or states[0] != self.initial_state:
            return 0
        for index in range(1, len(states)):
            if (states[index - 1], states[index]) not in self.transitions:
                return index
        return None

    def message_files(self):
        folder = os.path.join(self.fixtures, "messages")
        return [(name, os.path.join(folder, name)) for name in sorted(os.listdir(folder)) if name.endswith(".json")]


def source_bytes(fixture):
    """The bytes a fixture's source stands for (README section 13)."""
    if "value" in fixture:
        return canonical(fixture["value"]).encode("utf-8")
    if "text" in fixture:
        return fixture["text"].encode("utf-8", "surrogatepass")
    if "hex" in fixture:
        return bytes.fromhex(fixture["hex"])
    generate = fixture["generate"]
    return (generate["prefix"] + generate["fill"] * generate["count"] + generate["suffix"]).encode("utf-8")


# ---- The report (README section 13) ----


def report(contract):
    """The report lines, and what in them differs from the fixtures' expectations."""
    lines, problems = [], []
    read = lambda name: read_json(os.path.join(contract.fixtures, name))

    strict = read("strict-json.json")
    for listed in ("accept", "reject"):
        for entry in strict[listed]:
            try:
                text = canonical(parse_strict(source_bytes(entry)))
                lines.append("strict\t%s\taccept\t%s" % (entry["name"], sha256_hex(text)))
                if listed == "reject":
                    problems.append("strict %s: accepted" % entry["name"])
                elif text != entry["canonical"]:
                    problems.append("strict %s: canonical text differs" % entry["name"])
            except Refused as refusal:
                lines.append("strict\t%s\treject" % entry["name"])
                if listed == "accept":
                    problems.append("strict %s: refused (%s)" % (entry["name"], refusal))

    for reference, lists in read("scalars.json").items():
        for listed in ("accept", "reject"):
            for index, value in enumerate(lists[listed]):
                verdict = "accept" if contract.valid(reference, value) else "reject"
                lines.append("scalar\t%s\t%s\t%d\t%s" % (reference, listed, index, verdict))
                if verdict != listed:
                    problems.append("scalar %s %s[%d]" % (reference, listed, index))

    for entry in read("digests.json"):
        computed = digest(entry["domain"], entry["subject"])
        lines.append("digest\t%s\t%s" % (entry["name"], computed))
        if computed != entry["digest"] or canonical(entry["subject"]) != entry["canonical"]:
            problems.append("digest %s" % entry["name"])

    for entry in read("lifecycle.json"):
        first = contract.first_invalid(entry["states"])
        lines.append("lifecycle\t%s\t%s\t%s" % (entry["name"], "valid" if first is None else "invalid", "-" if first is None else first))
        if (first is None) != entry["valid"] or first != entry["first_invalid"]:
            problems.append("lifecycle %s" % entry["name"])

    for name, path in contract.message_files():
        fixture = read_json(path)
        expect = fixture["expect"]
        verdict = contract.judge(fixture["schema"], source_bytes(fixture))
        if verdict[0] == "accept":
            text = canonical(verdict[1])
            command, action = contract.frame_digests(fixture["schema"], verdict[1])
            lines.append("message\t%s\taccept\t%s\t%s\t%s" % (name, sha256_hex(text), command or "-", action or "-"))
            if expect != {"verdict": "accept", "canonical": text, "command_digest": command, "action_digest": action}:
                problems.append("message %s: the expectation differs" % name)
        else:
            lines.append("message\t%s\treject\t%s\t%s" % (name, verdict[1], verdict[2]))
            if expect != {"verdict": "reject", "layer": verdict[1], "error": verdict[2]}:
                problems.append("message %s: expected %s, computed %s" % (name, expect, verdict))

    return sorted(lines, key=lambda line: line.encode("utf-8")), problems


def judge_mutants(contract, path):
    lines = []
    with open(path, encoding="utf-8") as file:
        for line in file:
            if not line.strip():
                continue
            mutant = json.loads(line)
            verdict = contract.judge(mutant["schema"], mutant["text"].encode("utf-8", "surrogatepass"))
            if verdict[0] == "accept":
                lines.append("mutant\t%s\taccept" % mutant["id"])
            else:
                lines.append("mutant\t%s\treject\t%s\t%s" % (mutant["id"], verdict[1], verdict[2]))
    return sorted(lines, key=lambda line: line.encode("utf-8"))


# ---- Blessing: computing the expectations of the fixtures ----


def bless(contract):
    """Fills in what an accepted fixture expects. Returns what is wrong with the fixtures."""
    problems = []

    path = os.path.join(contract.fixtures, "strict-json.json")
    strict = read_json(path)
    for entry in strict["accept"]:
        try:
            entry["canonical"] = canonical(parse_strict(source_bytes(entry)))
        except Refused as refusal:
            problems.append("strict %s: meant to be accepted, but refused (%s)" % (entry["name"], refusal))
    write_json(path, strict)

    path = os.path.join(contract.fixtures, "digests.json")
    digests = read_json(path)
    for entry in digests:
        entry["canonical"] = canonical(entry["subject"])
        entry["digest"] = digest(entry["domain"], entry["subject"])
    write_json(path, digests)

    for name, path in contract.message_files():
        fixture = read_json(path)
        sources = [member for member in ("value", "text", "hex", "generate") if member in fixture]
        if len(sources) != 1 or list(fixture) != ["description", "schema", sources[0], "expect"]:
            problems.append("%s: the members must be description, schema, one source, expect" % name)
            continue
        expect = fixture["expect"]
        verdict = contract.judge(fixture["schema"], source_bytes(fixture))
        if expect.get("verdict") == "accept":
            if verdict[0] != "accept":
                problems.append("%s: meant to be accepted, but %s" % (name, verdict))
                continue
            command, action = contract.frame_digests(fixture["schema"], verdict[1])
            fixture["expect"] = {
                "verdict": "accept",
                "canonical": canonical(verdict[1]),
                "command_digest": command,
                "action_digest": action,
            }
            write_json(path, fixture)
        elif expect.get("verdict") == "reject":
            if verdict[0] != "reject" or expect != {"verdict": "reject", "layer": verdict[1], "error": verdict[2]}:
                problems.append("%s: expects %s, but computed %s" % (name, expect, verdict[:1] if verdict[0] == "accept" else verdict))
        else:
            problems.append("%s: expect has no verdict" % name)
    return problems


def main(arguments):
    here = os.path.dirname(os.path.abspath(__file__))
    default_root = os.path.dirname(here)
    if not arguments or arguments[0] not in ("report", "judge", "bless"):
        sys.stderr.write(__doc__)
        return 64
    command, rest = arguments[0], arguments[1:]
    if command == "judge":
        if not rest:
            sys.stderr.write(__doc__)
            return 64
        contract = Contract(rest[1] if len(rest) > 1 else default_root)
        sys.stdout.write("".join(line + "\n" for line in judge_mutants(contract, rest[0])))
        return 0
    contract = Contract(rest[0] if rest else default_root)
    if command == "report":
        lines, problems = report(contract)
        sys.stdout.write("".join(line + "\n" for line in lines))
    else:
        problems = bless(contract)
        print("blessed; problems: %d" % len(problems))
    for problem in problems:
        print("problem: " + problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    sys.exit(main(sys.argv[1:]))
