"""Advanced data pipeline with transitive dead chains, circular dead refs, and dead stores."""

import csv
import json


class Pipeline:
    """Live class with a mix of live and dead methods."""

    def __init__(self, source, transformers=None):
        self.source = source
        self.transformers = transformers or []
        self.errors = []
        self._cache = {}

    def run(self):
        """Live: called from main()."""
        raw = self._load_source()
        validated = self.validate(raw)
        result = self.transform(validated)
        return result

    def validate(self, records):
        """Live: called from run()."""
        valid = []
        for record in records:
            if record.get("id") and record.get("value") is not None:
                valid.append(record)
        return valid

    def transform(self, records):
        """Live: called from run()."""
        output = []
        for record in records:
            transformed = dict(record)
            for fn in self.transformers:
                transformed = fn(transformed)
            output.append(transformed)
        return output

    def _load_source(self):
        """Live: called from run()."""
        with open(self.source, "r") as f:
            return json.load(f)

    # --- Dead methods on this live class ---

    def export_csv(self, records, path):
        """Dead method: never called from run() or main(). Starts a 3-level dead chain."""
        rows = [_format_row(r) for r in records]
        with open(path, "w", newline="") as f:
            writer = csv.writer(f)
            writer.writerows(rows)

    def rollback(self):
        """Dead method: never called anywhere."""
        self._cache.clear()
        self.errors.clear()
        return True

    def export_json(self, records, path):
        """Dead method: never called anywhere."""
        formatted = [_format_row(r) for r in records]
        with open(path, "w") as f:
            json.dump(formatted, f, indent=2)


# --- Transitive dead chain (3 levels) ---


def _format_row(record):
    """Dead (level 2): only called from dead export_csv/export_json."""
    fields = []
    for key in sorted(record.keys()):
        fields.append(_escape_field(str(record[key])))
    return fields


def _escape_field(value):
    """Dead (level 3): only called from dead _format_row."""
    if "," in value or '"' in value or "\n" in value:
        return '"' + value.replace('"', '""') + '"'
    return value


# --- Circular dead references ---


def resolve_deps(modules):
    """Dead: mutually recursive with check_circular, neither called from main."""
    graph = {}
    for mod in modules:
        graph[mod["name"]] = mod.get("deps", [])
    for name in graph:
        if check_circular(graph, name, set()):
            return None
    return graph


def check_circular(graph, node, visited):
    """Dead: mutually recursive with resolve_deps, neither called from main."""
    if node in visited:
        return True
    visited.add(node)
    for dep in graph.get(node, []):
        if resolve_deps([{"name": dep, "deps": graph.get(dep, [])}]) is None:
            return True
    return False


# --- Dead callback pattern ---


def on_error_callback(error, context):
    """Dead callback: defined but never passed to run() or registered anywhere."""
    print(f"Error in {context}: {error}")
    return {"handled": True, "error": str(error)}


# --- Dead stores ---


def process_batch(items):
    """Dead function with multiple dead store patterns."""
    # Dead store: overwritten each iteration, only last value used
    total = 0
    for item in items:
        total = item.get("amount", 0)  # overwrites, doesn't accumulate

    # Dead store: return value captured but never used
    formatted_total = format(total, ".2f")

    # Dead store: assigned in both branches but only one branch executes
    if True:
        status = "complete"
    else:
        status = "pending"

    result = {"total": total, "status": status}
    return result


# --- Entry point ---


def main():
    """Entry point: only uses Pipeline.run(), not export or rollback."""
    transformers = [
        lambda r: {**r, "value": r["value"] * 2},
        lambda r: {**r, "processed": True},
    ]
    pipeline = Pipeline("data.json", transformers)
    results = pipeline.run()
    print(f"Processed {len(results)} records")
    return results


if __name__ == "__main__":
    main()
