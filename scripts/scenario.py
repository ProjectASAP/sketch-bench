#!/usr/bin/env python3

import decision
import rollout
import rules
import threshold_alert
import topk_set
from approx_rows import row

# For each application: the app_state metric, the values that are an action, and the label naming the entity.
ACTIONS = {
    threshold_alert.APP: ("alert", {threshold_alert.FIRING}, None),
    decision.APP: ("action", {"up", "down"}, "container"),
    rollout.APP: ("action", {rollout.ROLLBACK}, None),
    topk_set.APP: ("capped", {topk_set.CAPPED}, "client_id"),
    rules.APP: ("state", {rules.ACTIVE}, rules.KEY),
}


def change_text(change):
    if "scale" in change:
        return f"x{change['scale']['factor']:g}"
    if "rate" in change:
        return f"rate x{change['rate']['factor']:g}"
    return f"{change['mix']['share']:.0%} mix"


def incidents(spec, entity_of):
    """Every injected change that does not span the whole run, with the entities it touches.

    Phases of one series group over the same span are one incident. entity_of(labels) names a series'
    entity; None means the use case has no entities. A metric whose series come from values names its
    entities by the values its phases mix in.
    """
    out = []
    for metric in spec["metrics"]:
        for group in metric["series"]:
            spans = {}
            for phase in group.get("phases", []):
                if phase["start_offset_ms"] <= 0 and phase["end_offset_ms"] >= spec["duration_ms"]:
                    continue
                key = (phase["start_offset_ms"], phase["end_offset_ms"], phase.get("stagger_ms", 0))
                spans.setdefault(key, []).append(phase["change"])
            for (start, end, stagger), changes in spans.items():
                name = (f"{metric['name']} {' + '.join(change_text(c) for c in changes)} "
                        f"{start // 60000}-{end // 60000} min")
                if metric.get("label_from_value"):
                    mixed = [c["mix"]["distribution"] for c in changes if "mix" in c]
                    entities = [(str(v), 0) for d in mixed
                                for v in range(int(d.get("lower_bound", 0)), int(d.get("upper_bound", 0)))]
                else:
                    entities = []
                    for i in range(group.get("count", 1)):
                        labels = {k: v.replace("{i}", str(i)) for k, v in group["labels"].items()}
                        entities.append((entity_of(labels) if entity_of else None, i * stagger))
                for entity, shift in entities:
                    out.append({"name": name, "entity": entity, "start": spec["start_ms"] + start + shift,
                                "end": spec["start_ms"] + end + shift})
    return out


def actions(derived):
    """(app, signal) -> arm -> [(time, entity)] for every evaluation where the application acted."""
    out = {}
    for r in derived:
        if r["kind"] != "app_state" or "arm" not in r["labels"]:
            continue
        app = r["labels"].get("app")
        if app not in ACTIONS:
            continue
        metric, acting, key = ACTIONS[app]
        if r["metric"] != metric or r["value"] not in acting:
            continue
        signal = r["labels"].get("signal", r["labels"].get("query"))
        by_arm = out.setdefault((app, signal), {"exact": [], "approx": []})
        by_arm[r["labels"]["arm"]].append((r["time"], r["labels"].get(key) if key else None))
    return out


def covers(incident, time, entity, grace):
    if incident["entity"] is not None and entity is not None and incident["entity"] != entity:
        return False
    return incident["start"] < time <= incident["end"] + grace


def score(found, grace, acted):
    """Which incidents an arm's actions caught, after how long, and how many actions no incident explains."""
    by_name = {}
    for incident in found:
        by_name.setdefault(incident["name"], []).append(incident)
    caught, delays = 0, []
    for name, parts in by_name.items():
        hits = [t - part["start"] for part in parts for t, e in acted if covers(part, t, e, grace)]
        if hits:
            caught += 1
            delays.append(min(hits))
    outside = sum(1 for t, e in acted if not any(covers(i, t, e, grace) for i in found))
    out = {"incidents": len(by_name), "caught": caught, "actions_outside": outside}
    if delays:
        out["mean_detection_seconds"] = sum(delays) / len(delays) / 1000
    return out


def derive(run_id, spec, entity_of, derived):
    found = incidents(spec, entity_of)
    if not found:
        return [], {}
    grace = max(q["window_ms"] for q in spec["queries"])
    end = max(r["time"] for r in derived)
    rows, summary = [], {}
    for (app, signal), by_arm in sorted(actions(derived).items()):
        for arm, acted in by_arm.items():
            scored = score(found, grace, acted)
            summary.setdefault(signal, {})[arm] = scored
            labels = {"use_case": spec["use_case"], "app": app, "signal": signal, "arm": arm}
            rows += [row(end, "fidelity_summary", run_id, labels, f"scenario_{metric}", value,
                         "seconds" if metric.endswith("seconds") else "count")
                     for metric, value in scored.items()]
    return rows, summary
