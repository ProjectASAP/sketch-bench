#!/usr/bin/env python3

import json
import os

DS = {"type": "yesoreyeram-infinity-datasource", "uid": "approx-rows"}
DATA = "http://localhost:8000/runs/"
RUN = DATA + "${run}/"
INDEX = DATA + "index.json"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "dashboards")


def iq(ref, url, root="", columns=None, empty="[]"):
    return {"refId": ref, "datasource": DS, "type": "json", "source": "url", "parser": "backend",
            "format": "table", "url": url, "url_options": {"method": "GET", "data": ""},
            "root_selector": or_empty(root, empty) if root else "", "columns": columns or []}


def col(selector, text, type_):
    return {"selector": selector, "text": text, "type": type_}


T = col("time", "time", "timestamp_epoch")


def rows_of(ref, path, sel, name, type_="number", by=None, empty="[]"):
    columns = [T, col("value", name, type_)]
    root = f"$[{sel}]"
    if by:
        root = f'$[{sel}].{{"time": time, "value": value, "{by}": labels.{by}}}'
        columns.append(col(by, by, "string"))
    target = iq(ref, RUN + path + ".json", root, columns, empty)
    if by:
        target["format"] = "timeseries"
    return target


def or_empty(root, empty="[]"):
    return f"( $r := {root}; $exists($r) ? $r : {empty} )"


def metric_values(sel, body="$s"):
    return f"( $m := $[{sel}]; $exists($m) ? ( $s := $m{{metric: value}}; {body} ) : [] )"


def entity_file(kind, variable):
    return f"{kind}/${{{variable}}}"


def named(*names):
    return [{"matcher": {"id": "byName", "options": n}, "properties": [{"id": "displayName", "value": n}]}
            for n in names]


def by_ref(ref, color, width=1):
    return {"matcher": {"id": "byFrameRefID", "options": ref},
            "properties": [{"id": "color", "value": {"mode": "fixed", "fixedColor": color}},
                           {"id": "custom.lineWidth", "value": width}]}


def dashed(ref, color):
    return {"matcher": {"id": "byFrameRefID", "options": ref},
            "properties": [{"id": "color", "value": {"mode": "fixed", "fixedColor": color}},
                           {"id": "custom.lineStyle", "value": {"fill": "dash", "dash": [10, 10]}}]}


def order(*names):
    return [{"id": "organize", "options": {"indexByName": {n: i for i, n in enumerate(names)},
                                           "excludeByName": {}, "renameByName": {}}}]


def in_list(values):
    return "[" + ", ".join(f'"{v}"' for v in values) + "]"


LINES = {"custom": {"lineWidth": 1, "showPoints": "never"}}
LEGEND = {"legend": {"displayMode": "list", "placement": "bottom", "showLegend": True},
          "tooltip": {"mode": "multi", "sort": "none"}}
STAT = {"reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
        "textMode": "value_and_name", "colorMode": "none", "graphMode": "none",
        "orientation": "horizontal", "justifyMode": "auto"}
TEXT = {"color": {"mode": "fixed", "fixedColor": "text"}}
OFF = {"color": "transparent", "text": " "}
ANNOTATION = {"none": OFF, "onset delay": {"color": "#FADE2A"}, "gap": {"color": "#8F3BB8"},
              "end delay": {"color": "#19B5B5"}, "false positive": {"color": "#FF9830"},
              "false negative": {"color": "#D3369E"}, "missed episode": {"color": "#F5A3C7"},
              "decision delay": {"color": "#A0703C"}}
COMBINED = "#9E9E9E"
OUTCOMES = {"true_positive": "true positive", "false_positive": "false positive",
            "false_negative": "false negative", "true_negative": "true negative"}


def value_mappings(states):
    options = {value: {**result, "index": i} for i, (value, result) in enumerate(states.items())}
    return [{"type": "value", "options": options},
            {"type": "regex", "options": {"pattern": ".+, .+", "result": {"color": COMBINED, "index": len(states)}}}]


def strip_panel(title, rows, states, path="app_state"):
    return {"type": "state-timeline", "title": title,
            "targets": [rows_of(chr(ord("A") + i), path, sel, name, "string",
                                empty='( $exists($$[0]) ? [{"time": $$[0].time}] : [] )')
                        for i, (sel, name) in enumerate(rows)],
            "fieldConfig": {"defaults": {"custom": {"fillOpacity": 80, "lineWidth": 0},
                                         "color": {"mode": "fixed", "fixedColor": "transparent"},
                                         "mappings": value_mappings({**ANNOTATION, **states})},
                            "overrides": named(*[name for _, name in rows])},
            "options": {"mergeValues": False, "showValue": "auto", "rowHeight": 0.8, "alignValue": "center",
                        **LEGEND, "legend": {"showLegend": False}}}


def outcome_stat(title, summary, sel, units=None):
    names = {metric: f"{text} ({units[metric]})" if units else text for metric, text in OUTCOMES.items()}
    return {"type": "stat", "title": title,
            "targets": [iq("A", summary, metric_values(sel),
                           by_metric_columns(*names.items()))],
            "transformations": order(*names.values()),
            "fieldConfig": {"defaults": TEXT, "overrides": []},
            "options": STAT}


def disagreement_panel(uid, name, key, title):
    return {"type": "table", "title": title,
            "targets": [iq("A", RUN + "fidelity_summary.json",
                           f'$[metric="disagreements" and value > 0].'
                           f'{{"{key}": labels.{key}, "disagreements": value}}',
                           [col(key, key, "string"), col("disagreements", "disagreements", "number")])],
            "transformations": sort_limit("disagreements", 20),
            "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": [
                {"matcher": {"id": "byName", "options": key},
                 "properties": [{"id": "links", "value": [{
                     "title": f"Show this {name}",
                     "url": f"/d/{uid}/{uid}?var-run=${{run}}&var-{name}=${{__data.fields.{key}}}"
                            "&${__url_time_range}"}]}]}]},
            "options": {"showHeader": True}}


def by_disagreements(name, key):
    ranked = ('( $s := $sort($[metric="disagreements"], function($a, $b) { $a.value < $b.value or '
              f'($a.value = $b.value and $a.labels.{key} > $b.labels.{key}) }}); $s.labels )')
    out = variable(name, RUN + "fidelity_summary.json", ranked, key)
    out["sort"] = 0
    return out


def in_run(name, key, chosen):
    root = (f'( $m := $[metric="disagreements" and labels.{key} in [${{{chosen}:doublequote}}]].labels; '
            f'$a := $[metric="disagreements"].labels; $exists($m) ? $m : $a[0] )')
    out = variable(name, RUN + "fidelity_summary.json", root, key)
    out.update(hide=2, skipUrlSync=True)
    return out


class Board:
    def __init__(self):
        self.panels = []

    def _id(self):
        return len(self.panels) + 1

    def panel(self, p, x, y, w, h):
        p.update(id=self._id(), gridPos={"x": x, "y": y, "w": w, "h": h})
        p.setdefault("datasource", DS)
        self.panels.append(p)

    def row(self, title, y):
        self.panels.append({"id": self._id(), "type": "row", "title": title, "collapsed": False,
                            "gridPos": {"x": 0, "y": y, "w": 24, "h": 1}, "panels": []})


def runs_panel(board, uid, use_cases, x, y, w, h):
    board.panel({"type": "table", "title": "Runs",
                 "targets": [iq("A", INDEX, f"$[use_case in {in_list(use_cases)}]",
                                [col("run_id", "run_id", "string"), col("use_case", "use_case", "string"),
                                 col("start", "start", "number"), col("end", "end", "number"),
                                 col("details.arm.max_error", "max_error", "string"),
                                 col("details.arm.raw_data_retained", "raw_data_retained", "string"),
                                 col("details.arm.ignore_guarantees", "ignore_guarantees", "string"),
                                 col("details.planner_rev", "planner_rev", "string")])],
                 "fieldConfig": {"defaults": {}, "overrides": [
                     {"matcher": {"id": "byRegexp", "options": "start|end"},
                      "properties": [{"id": "unit", "value": "dateTimeAsIso"}]},
                     {"matcher": {"id": "byName", "options": "run_id"},
                      "properties": [{"id": "links", "value": [{
                          "title": "Open this run",
                          "url": f"/d/{uid}/{uid}?var-run=${{__data.fields.run_id}}"
                                 "&from=${__data.fields.start}&to=${__data.fields.end}"}]}]}]},
                 "transformations": order("run_id", "max_error", "raw_data_retained", "ignore_guarantees",
                                          "use_case", "start", "end", "planner_rev"),
                 "options": {"showHeader": True}}, x, y, w, h)


def plans_panel(board, x, y, w, h):
    each = ('( $e := $[run_id="${run}"]; $append('
            '$e.details.plans ~> $each(function($v, $k) { {"query": $k, "approx plan": $v, '
            '"exact plan": $lookup($e.details.exact_plans, $k), "promql": $lookup($e.details.promql, $k)} }), '
            '$e.details.unsupported ~> $each(function($v, $k) { {"query": $k, "why no approx plan": $v, '
            '"promql": $lookup($e.details.promql, $k)} })) )')
    board.panel({"type": "table", "title": "Plans in ${run}",
                 "targets": [iq("A", INDEX, each,
                                [col("query", "query", "string"), col("approx plan", "approx plan", "string"),
                                 col("exact plan", "exact plan", "string"),
                                 col("why no approx plan", "why no approx plan", "string"),
                                 col("promql", "promql", "string")])],
                 "transformations": order("query", "approx plan", "exact plan", "why no approx plan", "promql"),
                 "fieldConfig": {"defaults": {"custom": {"cellOptions": {"type": "auto", "wrapText": True}}},
                                 "overrides": []},
                 "options": {"showHeader": True}}, x, y, w, h)


def run_variable(use_cases):
    return {"name": "run", "label": "run", "type": "query", "datasource": DS, "refresh": 1,
            "query": {"queryType": "infinity", "query": "", "refId": "variable",
                      "infinityQuery": iq("variable", INDEX, f"$[use_case in {in_list(use_cases)}]",
                                          [col("run_id", "run_id", "string")])}}


def variable(name, url, root, selector):
    return {"name": name, "label": name, "type": "query", "datasource": DS, "refresh": 1,
            "query": {"queryType": "infinity", "query": "", "refId": "variable",
                      "infinityQuery": iq("variable", url, root, [col(selector, name, "string")])}}


def dashboard(uid, title, board, variables, annotations=()):
    return {
        "uid": uid,
        "title": title,
        "tags": ["approx-rows"],
        "timezone": "utc",
        "editable": False,
        "schemaVersion": 41,
        "time": {"from": "2026-01-01T00:00:00.000Z", "to": "2026-01-01T01:00:00.000Z"},
        "templating": {"list": variables},
        "annotations": {"list": [
            {"name": "Annotations & Alerts", "builtIn": 1, "enable": True, "hide": True,
             "iconColor": "rgba(0, 211, 255, 1)",
             "datasource": {"type": "grafana", "uid": "-- Grafana --"}, "type": "dashboard"},
            *annotations]},
        "panels": board.panels,
    }


def by_metric_columns(*pairs):
    return [col(selector, text, "number") for selector, text in pairs]


def sort_limit(field, limit):
    return [{"id": "sortBy", "options": {"sort": [{"field": field, "desc": True}]}},
            {"id": "limit", "options": {"limitField": limit}}]


def alert_dashboard():
    uid = "threshold-alert"
    use_cases = ["ddos_syn_flood"]
    sel = 'labels.signal="${signal}"'
    b = Board()
    b.row("Runs", 0)
    runs_panel(b, uid, use_cases, 0, 1, 12, 7)
    plans_panel(b, 12, 1, 12, 7)

    b.row("Query result", 8)
    b.panel({"type": "timeseries", "title": "Queries behind ${signal}: exact vs approx",
             "targets": [rows_of("A", "query_result", f'{sel} and labels.arm="exact"', "exact", by="query"),
                         rows_of("B", "query_result", f'{sel} and labels.arm="approx"', "approx", by="query"),
                         rows_of("C", "app_state", f'{sel} and metric="threshold"', "threshold")],
             "fieldConfig": {"defaults": {**LINES, "displayName": "${__field.labels.query} ${__field.name}"},
                             "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange"), dashed("C", "red")]
                             + named("threshold")},
             "options": LEGEND}, 0, 9, 24, 9)
    b.panel({"type": "timeseries", "title": "Relative error of approx",
             "targets": [rows_of("A", "low_level", f'{sel} and metric="relative_error"', "relative_error", by="query")],
             "fieldConfig": {"defaults": {**LINES, "unit": "percentunit", "displayName": "${__field.labels.query}"},
                             "overrides": []},
             "options": LEGEND}, 0, 18, 24, 6)

    b.row("Application: alert", 24)
    b.panel(strip_panel("${signal}: alert per evaluation, with what differs",
                        [(f'{sel} and metric="alert" and labels.arm="exact"', "exact alert"),
                         (f'{sel} and metric="alert" and labels.arm="approx"', "approx alert"),
                         (f'{sel} and metric="annotation"', "annotation")],
                        {"firing": {"color": "red", "text": "firing"}, "ok": OFF}), 0, 25, 24, 7)

    b.row("Fidelity", 32)
    summary = RUN + "fidelity_summary.json"
    by_metric = metric_values(sel)
    b.panel(outcome_stat("Evaluations by outcome", summary, sel), 0, 33, 6, 8)
    b.panel({"type": "stat", "title": "Episodes",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("episodes", "exact episodes"), ("missed_episodes", "missed episodes"),
                 ("mean_onset_delay", "mean onset delay (evaluations)"),
                 ("mean_absolute_end_delay", "mean absolute end delay (evaluations)"),
                 ("early_ends", "early ends"), ("late_ends", "late ends"), ("gaps", "gaps"),
                 ("episode_coverage", "episode coverage")))],
             "transformations": order("exact episodes", "missed episodes", "mean onset delay (evaluations)",
                                      "mean absolute end delay (evaluations)", "early ends", "late ends", "gaps",
                                      "episode coverage"),
             "fieldConfig": {"defaults": TEXT, "overrides": [
                 {"matcher": {"id": "byName", "options": "episode coverage"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1}]}]},
             "options": STAT}, 6, 33, 6, 8)
    b.panel({"type": "table", "title": "Confusion matrix (evaluations)",
             "targets": [iq("A", summary,
                            metric_values(sel, '['
                            '{"exact": "firing", "approx firing": $s.true_positive, "approx ok": $s.false_negative}, '
                            '{"exact": "ok", "approx firing": $s.false_positive, "approx ok": $s.true_negative}]'),
                            [col("exact", "exact", "string"), col("approx firing", "approx firing", "number"),
                             col("approx ok", "approx ok", "number")])],
             "transformations": order("exact", "approx firing", "approx ok"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 12, 33, 6, 8)
    b.panel({"type": "table", "title": "Fidelity events",
             "targets": [iq("A", RUN + "fidelity_event.json",
                            f"$[{sel}]",
                            [T, col("metric", "event", "string"), col("value", "evaluations", "number")])],
             "transformations": order("time", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 18, 33, 6, 8)

    variables = [run_variable(use_cases),
                 variable("signal", summary, '$[metric="episodes"].labels', "signal")]
    annotations = [{"name": "fidelity events", "enable": True, "iconColor": "purple", "datasource": DS,
                    "target": iq("Anno", RUN + "fidelity_event.json",
                                 f'$[{sel}].{{"time": time, "text": metric & ": " & $string(value) & " evaluations"}}',
                                 [T, col("text", "text", "string")])}]
    return dashboard(uid, "Threshold alert", b, variables, annotations)


def decision_dashboard():
    uid = "decision"
    use_cases = ["k8s_autoscaling"]
    here = 'labels.container="${container}"'
    overall = 'labels.app="autoscaler" and $not($exists(labels.container))'
    summary = RUN + "fidelity_summary.json"
    b = Board()
    b.row("Runs", 0)
    runs_panel(b, uid, use_cases, 0, 1, 12, 7)
    plans_panel(b, 12, 1, 12, 7)

    b.row("Query result for ${container}", 8)
    for i, (query, title, unit, low, high) in enumerate((("p95_cpu", "p95 CPU", "none", 0.4, 0.8),
                                                         ("p95_memory", "p95 memory", "bytes", 0.75e9, 1.5e9))):
        sel = f'{here} and labels.query="{query}"'
        results = entity_file("query_result", "container_in_run")
        b.panel({"type": "timeseries", "title": f"{title} of ${{container}}: exact vs approx",
                 "targets": [rows_of("A", results, f'{sel} and labels.arm="exact"', "exact"),
                             rows_of("B", results, f'{sel} and labels.arm="approx"', "approx")],
                 "fieldConfig": {"defaults": {**LINES, "unit": unit,
                                              "custom": {**LINES["custom"], "thresholdsStyle": {"mode": "dashed"}},
                                              "thresholds": {"mode": "absolute", "steps": [
                                                  {"color": "blue", "value": None}, {"color": "green", "value": low},
                                                  {"color": "red", "value": high}]}},
                                 "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange")] + named("exact", "approx")},
                 "options": LEGEND}, 12 * i, 9, 12, 8)
    b.panel({"type": "timeseries", "title": "Relative error of approx for ${container}",
             "targets": [rows_of("A", entity_file("low_level", "container_in_run"), f'{here} and metric="relative_error"',
                                 "relative_error", by="query")],
             "fieldConfig": {"defaults": {**LINES, "unit": "percentunit", "displayName": "${__field.labels.query}"},
                             "overrides": []},
             "options": LEGEND}, 0, 17, 24, 6)

    b.row("Application: autoscaler", 23)
    scaling = {"up": {"color": "red", "text": "scale up"}, "down": {"color": "blue", "text": "scale down"}}
    b.panel(strip_panel("${container}: scaling action per evaluation, with what differs",
                        [(f'{here} and metric="action" and labels.arm="exact"', "exact action"),
                         (f'{here} and metric="action" and labels.arm="approx"', "approx action"),
                         (f'{here} and metric="annotation"', "annotation")], scaling,
                        entity_file("app_state", "container_in_run")), 0, 24, 24, 6)
    b.panel(strip_panel("${container}: scaling decision per evaluation",
                        [(f'{here} and metric="decision" and labels.arm="exact"', "exact decision"),
                         (f'{here} and metric="decision" and labels.arm="approx"', "approx decision")],
                        {**scaling, "none": {"color": "green", "text": "no change"}},
                        entity_file("app_state", "container_in_run")), 0, 30, 24, 5)

    b.row("Fidelity over all containers", 35)
    b.panel(outcome_stat("Scaling outcomes", summary, overall,
                         {"true_positive": "actions", "false_positive": "actions", "false_negative": "actions",
                          "true_negative": "container-evaluations"}), 0, 36, 6, 7)
    fields = (("true_positive", "matched"), ("early_actions", "early"), ("late_actions", "late"),
              ("false_positive", "false"), ("false_negative", "missed"),
              ("mean_absolute_onset_delay", "mean absolute onset delay"))
    per_action = metric_values(overall, '["up", "down"].( $a := $; {"action": $a, '
                               + ", ".join(f'"{text}": $lookup($s, $a & "_{metric}")' for metric, text in fields)
                               + "} )")
    b.panel({"type": "table", "title": "Scaling actions by kind",
             "targets": [iq("A", summary, per_action, [col("action", "action", "string")]
                            + [col(text, text, "number") for _, text in fields])],
             "transformations": order("action", *[text for _, text in fields]),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 6, 36, 7, 7)
    matrix = metric_values(overall, '["up", "down", "none"].( $e := $; {"exact": $e, '
                           + ", ".join(f'"approx {a}": $lookup($s, "exact_" & $e & "_approx_{a}")'
                                       for a in ("up", "down", "none"))
                           + "} )")
    b.panel({"type": "table", "title": "Decisions, exact by approx (container-evaluations)",
             "targets": [iq("A", summary, matrix, [col("exact", "exact", "string")]
                            + [col(f"approx {a}", f"approx {a}", "number") for a in ("up", "down", "none")])],
             "transformations": order("exact", "approx up", "approx down", "approx none"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 13, 36, 6, 7)
    b.panel(disagreement_panel(uid, "container", "container", "Containers with the most action disagreements"), 19, 36, 5, 7)
    b.panel({"type": "table", "title": "Fidelity events of ${container}",
             "targets": [iq("A", RUN + "fidelity_event.json", f"$[{here}]",
                            [T, col("labels.action", "action", "string"), col("metric", "event", "string"),
                             col("value", "evaluations", "number")])],
             "transformations": order("time", "action", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 0, 43, 24, 6)

    annotations = [{"name": "fidelity events", "enable": True, "iconColor": "purple", "datasource": DS,
                    "target": iq("Anno", RUN + "fidelity_event.json",
                                 f'$[{here}].{{"time": time, "text": labels.action & " " & metric & ": " '
                                 '& $string(value) & " evaluations"}',
                                 [T, col("text", "text", "string")])}]
    return dashboard(uid, "Autoscaler decision", b,
                     [run_variable(use_cases), by_disagreements("container", "container"),
                      in_run("container_in_run", "container", "container")], annotations)


def rollout_dashboard():
    uid = "rollout"
    use_cases = ["canary_release"]
    app = 'labels.app="rollout_controller"'
    summary = RUN + "fidelity_summary.json"
    b = Board()
    b.row("Runs", 0)
    runs_panel(b, uid, use_cases, 0, 1, 12, 7)
    plans_panel(b, 12, 1, 12, 7)

    b.row("Query result", 8)
    b.panel({"type": "timeseries", "title": "p99 latency of canary and stable: exact vs approx",
             "targets": [rows_of("A", "query_result", 'labels.arm="exact"', "exact", by="query"),
                         rows_of("B", "query_result", 'labels.arm="approx"', "approx", by="query")],
             "fieldConfig": {"defaults": {**LINES, "unit": "s", "displayName": "${__field.labels.query} ${__field.name}"},
                             "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange")]},
             "options": LEGEND}, 0, 9, 24, 8)
    b.panel({"type": "timeseries", "title": "Relative error of approx",
             "targets": [rows_of("A", "low_level", 'metric="relative_error"', "relative_error", by="query")],
             "fieldConfig": {"defaults": {**LINES, "unit": "percentunit", "displayName": "${__field.labels.query}"},
                             "overrides": []},
             "options": LEGEND}, 0, 17, 24, 6)

    b.row("Application: rollout controller", 23)
    b.panel({"type": "timeseries", "title": "Canary p99 over stable p99",
             "targets": [rows_of("A", "app_state", f'{app} and metric="ratio" and labels.arm="exact"', "exact"),
                         rows_of("B", "app_state", f'{app} and metric="ratio" and labels.arm="approx"', "approx"),
                         rows_of("C", "app_state", f'{app} and metric="tolerance"', "tolerance")],
             "fieldConfig": {"defaults": LINES,
                             "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange"), dashed("C", "red")]
                             + named("exact", "approx", "tolerance")},
             "options": LEGEND}, 0, 24, 24, 7)
    b.panel(strip_panel("Slower check and rollout decision per evaluation, with what differs",
                        [(f'{app} and metric="slower" and labels.arm="exact"', "exact slower"),
                         (f'{app} and metric="slower" and labels.arm="approx"', "approx slower"),
                         (f'{app} and metric="action" and labels.arm="exact"', "exact action"),
                         (f'{app} and metric="action" and labels.arm="approx"', "approx action"),
                         (f'{app} and metric="annotation"', "annotation")],
                        {"slower": {"color": "red", "text": "slower"}, "not slower": OFF,
                         "rollback": {"color": "red", "text": "rollback"},
                         "promote": {"color": "green", "text": "promote"}}), 0, 31, 24, 9)

    b.row("Fidelity over rollouts", 40)
    b.panel(outcome_stat("Rollouts by outcome (positive: rollback)", summary, app), 0, 41, 6, 8)
    b.panel({"type": "stat", "title": "Rollouts",
             "targets": [iq("A", summary, metric_values(app), by_metric_columns(
                 ("rollouts", "rollouts"), ("agreement", "same decision"),
                 ("mean_absolute_rollback_delay", "mean absolute rollback delay (evaluations)"),
                 ("early_rollbacks", "early rollbacks"), ("late_rollbacks", "late rollbacks"),
                 ("gaps", "gaps in slower")))],
             "transformations": order("rollouts", "same decision", "mean absolute rollback delay (evaluations)",
                                      "early rollbacks", "late rollbacks", "gaps in slower"),
             "fieldConfig": {"defaults": TEXT, "overrides": [
                 {"matcher": {"id": "byName", "options": "same decision"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1}]}]},
             "options": STAT}, 6, 41, 6, 8)
    b.panel({"type": "table", "title": "Rollouts",
             "targets": [iq("A", RUN + "fidelity_event.json", f'$[{app} and metric="rollout"]',
                            [T, col("labels.rollout", "rollout", "string"), col("labels.exact", "exact", "string"),
                             col("labels.approx", "approx", "string"), col("value", "outcome", "string")])],
             "transformations": order("time", "rollout", "exact", "approx", "outcome"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": [
                 {"matcher": {"id": "byName", "options": "outcome"},
                  "properties": [{"id": "mappings", "value": [{"type": "value", "options": {
                      value: {"text": text, "index": i} for i, (value, text) in enumerate(OUTCOMES.items())}}]}]}]},
             "options": {"showHeader": True}}, 12, 41, 7, 8)
    b.panel({"type": "table", "title": "Fidelity events",
             "targets": [iq("A", RUN + "fidelity_event.json", f'$[{app} and metric!="rollout"]',
                            [T, col("metric", "event", "string"), col("value", "evaluations", "number")])],
             "transformations": order("time", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 19, 41, 5, 8)
    return dashboard(uid, "Canary rollout", b, [run_variable(use_cases)])


def topk_dashboard():
    uid = "topk"
    use_cases = ["top_clients"]
    app = 'labels.app="topk_set"'
    here = f'{app} and labels.client_id="${{client_in_run}}"'
    overall = f'{app} and $not($exists(labels.client_id))'
    summary = RUN + "fidelity_summary.json"
    b = Board()
    b.row("Runs", 0)
    runs_panel(b, uid, use_cases, 0, 1, 12, 7)
    plans_panel(b, 12, 1, 12, 7)

    b.row("Query result", 8)
    b.panel({"type": "table", "title": "Top clients by evaluation, highest count first",
             "targets": [rows_of("A", "app_state", f'{app} and metric="ranking" and labels.arm="exact"', "exact", "string"),
                         rows_of("B", "app_state", f'{app} and metric="ranking" and labels.arm="approx"', "approx", "string")],
             "transformations": [{"id": "joinByField", "options": {"byField": "time", "mode": "outer"}},
                                 {"id": "organize", "options": {"indexByName": {"time": 0, "exact A": 1, "approx B": 2},
                                                                "excludeByName": {},
                                                                "renameByName": {"exact A": "exact", "approx B": "approx"}}}]
             + [{"id": "sortBy", "options": {"sort": [{"field": "time", "desc": True}]}}],
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 0, 9, 14, 9)
    b.panel({"type": "timeseries", "title": "Relative error of matched clients' counts",
             "targets": [rows_of("A", "low_level", 'metric="relative_error"', "relative_error", by="client_id")],
             "fieldConfig": {"defaults": {**LINES, "unit": "percentunit", "displayName": "${__field.labels.client_id}",
                                          "custom": {**LINES["custom"], "showPoints": "always", "pointSize": 3}},
                             "overrides": []},
             "options": {**LEGEND, "legend": {"showLegend": False}}}, 14, 9, 10, 9)

    b.row("Application: rate limiter", 18)
    b.panel({"type": "timeseries", "title": "Clients shared by the exact and approx top lists",
             "targets": [rows_of("A", "app_state", f'{app} and metric="overlap"', "in both"),
                         rows_of("B", "app_state", f'{app} and metric="approx_only"', "approx only")],
             "fieldConfig": {"defaults": {**LINES, "min": 0},
                             "overrides": [by_ref("A", "green", 2), by_ref("B", "orange")] + named("in both", "approx only")},
             "options": LEGEND}, 0, 19, 18, 7)
    b.panel(disagreement_panel(uid, "client", "client_id", "Clients with the most disagreements"), 18, 19, 6, 7)
    strip = strip_panel("Client ${client_in_run}: capped per evaluation, with what differs",
                        [(f'{here} and metric="capped" and labels.arm="exact"', "exact capped"),
                         (f'{here} and metric="capped" and labels.arm="approx"', "approx capped"),
                         (f'{here} and metric="annotation"', "annotation")],
                        {"capped": {"color": "red", "text": "capped"}, "not capped": OFF},
                        entity_file("app_state", "client_in_run"))
    strip.update(repeat="client_in_run", repeatDirection="v")
    b.panel(strip, 0, 26, 24, 5)

    b.row("Fidelity", 31)
    b.panel(outcome_stat("Client-evaluations by outcome", summary, overall), 0, 32, 6, 8)
    by_metric = metric_values(overall)
    b.panel({"type": "stat", "title": "Top list per evaluation",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("precision", "precision"), ("recall", "recall"), ("identical_sets", "identical lists"),
                 ("evaluations", "evaluations"), ("intruder_slots", "approx-only slots")))],
             "transformations": order("precision", "recall", "identical lists", "evaluations", "approx-only slots"),
             "fieldConfig": {"defaults": TEXT, "overrides": [
                 {"matcher": {"id": "byRegexp", "options": "precision|recall"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1}]}]},
             "options": STAT}, 6, 32, 6, 8)
    b.panel({"type": "stat", "title": "Clients entering the exact top list",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("entries", "entries"), ("missed_entries", "missed"),
                 ("mean_onset_delay", "mean onset delay (evaluations)"), ("gaps", "gaps"),
                 ("false_member_spans", "approx-only spans")))],
             "transformations": order("entries", "missed", "mean onset delay (evaluations)", "gaps",
                                      "approx-only spans"),
             "fieldConfig": {"defaults": TEXT, "overrides": []},
             "options": STAT}, 12, 32, 6, 8)
    b.panel({"type": "table", "title": "Fidelity events",
             "targets": [iq("A", RUN + "fidelity_event.json", f"$[{app}]",
                            [T, col("labels.client_id", "client", "string"), col("metric", "event", "string"),
                             col("value", "evaluations", "number")])],
             "transformations": order("time", "client", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 18, 32, 6, 8)
    client = by_disagreements("client", "client_id")
    client.update(multi=True, includeAll=False)
    shown = in_run("client_in_run", "client_id", "client")
    shown.update(multi=True, includeAll=True, current={"text": ["All"], "value": ["$__all"]})
    return dashboard(uid, "Top clients", b, [run_variable(use_cases), client, shown])


AGREEMENT = ("Defined per application. Threshold alert: evaluations where exact and approx agree on firing. "
             "Autoscaler: matched scaling actions over matched, false and missed ones. "
             "Rollout controller: rollouts decided the same. "
             "Top clients: mean clients shared by the exact and approx top lists, as a fraction of the list size.")


def overview_dashboard():
    rows = ('$[$exists(details.fidelity)].( $e := $; '
            '$append($keys($e.details.plans), $keys($e.details.unsupported)).( $q := $; '
            '$s := $lookup($e.details.signals, $q); $f := $lookup($e.details.fidelity, $s); '
            '{"run_id": $e.run_id, "use_case": $e.use_case, "start": $e.start, "end": $e.end, "query": $q, '
            '"approx plan": $lookup($e.details.plans, $q), '
            '"why no approx plan": $lookup($e.details.unsupported, $q), '
            '"max relative error": $lookup($e.details.max_relative_error, $q), '
            '"signal": $s, "agreement": $f.agreement, "fidelity": $f.fidelity, '
            '"dashboard": $e.details.dashboard} ) )')
    text = ("use_case", "run_id", "query", "approx plan", "why no approx plan", "signal", "fidelity", "dashboard")
    numbers = ("start", "end", "max relative error", "agreement")
    shown = ("use_case", "run_id", "query", "approx plan", "why no approx plan", "max relative error", "signal",
             "agreement", "fidelity")
    b = Board()
    b.panel({"type": "table", "title": "Every run and query: plan, query error, and what the application saw",
             "targets": [iq("A", INDEX, rows, [col(k, k, "string") for k in text]
                            + [col(k, k, "number") for k in numbers])],
             "transformations": [{"id": "organize", "options": {
                 "indexByName": {k: i for i, k in enumerate(shown)},
                 "excludeByName": {}, "renameByName": {}}},
                 {"id": "sortBy", "options": {"sort": [{"field": "run_id"}]}}],
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60, "cellOptions": {"type": "auto", "wrapText": True}}},
                             "overrides": [
                 {"matcher": {"id": "byRegexp", "options": "^(dashboard|start|end|use_case)$"},
                  "properties": [{"id": "custom.hidden", "value": True}]},
                 {"matcher": {"id": "byName", "options": "max relative error"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1},
                                 {"id": "custom.width", "value": 110}]},
                 {"matcher": {"id": "byName", "options": "agreement"},
                  "properties": [{"id": "description", "value": AGREEMENT},
                                 {"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1},
                                 {"id": "min", "value": 0}, {"id": "max", "value": 1},
                                 {"id": "custom.width", "value": 110},
                                 {"id": "custom.cellOptions", "value": {"type": "color-background", "mode": "basic"}},
                                 {"id": "color", "value": {"mode": "continuous-RdYlGr"}},
                                 {"id": "mappings", "value": [{"type": "special", "options": {
                                     "match": "null+nan", "result": {"text": "no plan", "color": "transparent", "index": 0}}}]}]},
                 {"matcher": {"id": "byName", "options": "approx plan"},
                  "properties": [{"id": "custom.width", "value": 210}]},
                 {"matcher": {"id": "byName", "options": "why no approx plan"},
                  "properties": [{"id": "custom.width", "value": 150}]},
                 {"matcher": {"id": "byRegexp", "options": "query|signal"},
                  "properties": [{"id": "custom.width", "value": 115}]},
                 {"matcher": {"id": "byName", "options": "run_id"},
                  "properties": [{"id": "custom.width", "value": 170}, {"id": "links", "value": [{
                      "title": "Open this run",
                      "url": "/d/${__data.fields.dashboard}/run?var-run=${__data.fields.run_id}"
                             "&var-signal=${__data.fields.signal}"
                             "&from=${__data.fields.start}&to=${__data.fields.end}"}]}]}]},
             "options": {"showHeader": True}}, 0, 0, 24, 30)
    return dashboard("overview", "Approximate query fidelity overview", b, [])


def main():
    for board in (overview_dashboard(), alert_dashboard(), decision_dashboard(), rollout_dashboard(),
                  topk_dashboard()):
        with open(os.path.join(OUT, board["uid"] + ".json"), "w") as f:
            json.dump(board, f, indent=2)
            f.write("\n")


if __name__ == "__main__":
    main()
