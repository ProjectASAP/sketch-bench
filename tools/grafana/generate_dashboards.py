#!/usr/bin/env python3

import json
import os

DS = {"type": "yesoreyeram-infinity-datasource", "uid": "approx-rows"}
DATA = "http://localhost:8000/runs/"
RUN = DATA + "${run}/"
INDEX = DATA + "index.json"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "dashboards")


def iq(ref, url, root="", columns=None):
    return {"refId": ref, "datasource": DS, "type": "json", "source": "url", "parser": "backend",
            "format": "table", "url": url, "url_options": {"method": "GET", "data": ""},
            "root_selector": root, "columns": columns or []}


def col(selector, text, type_):
    return {"selector": selector, "text": text, "type": type_}


T = col("time", "time", "timestamp_epoch")


def rows_of(ref, kind, sel, name, type_="number", by=None):
    columns = [T, col("value", name, type_)]
    root = f"$[{sel}]"
    if by:
        root = f'$[{sel}].{{"time": time, "value": value, "{by}": labels.{by}}}'
        columns.append(col(by, by, "string"))
    target = iq(ref, RUN + kind + ".json", or_empty(root), columns)
    if by:
        target["format"] = "timeseries"
    return target


def or_empty(root):
    return f"( $r := {root}; $exists($r) ? $r : [] )"


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


def alert_dashboard():
    uid = "threshold-alert"
    use_cases = ["ddos_syn_flood", "canary_release"]
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
    b.panel({"type": "timeseries", "title": "Compared ratio (comparison alerts only)",
             "targets": [rows_of("A", "app_state", f'{sel} and metric="ratio" and labels.arm="exact"', "exact"),
                         rows_of("B", "app_state", f'{sel} and metric="ratio" and labels.arm="approx"', "approx"),
                         rows_of("C", "app_state", f'{sel} and metric="tolerance"', "tolerance")],
             "fieldConfig": {"defaults": LINES,
                             "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange"), dashed("C", "red")]
                             + named("exact", "approx", "tolerance")},
             "options": LEGEND}, 0, 25, 24, 7)
    b.panel({"type": "state-timeline", "title": "${signal}: alert state",
             "targets": [rows_of("A", "app_state", f'{sel} and metric="alert" and labels.arm="exact"', "exact", "string"),
                         rows_of("B", "app_state", f'{sel} and metric="alert" and labels.arm="approx"', "approx", "string")],
             "fieldConfig": {"defaults": {"custom": {"fillOpacity": 80, "lineWidth": 0},
                                          "color": {"mode": "fixed", "fixedColor": "transparent"},
                                          "mappings": [{"type": "value", "options": {
                                              "firing": {"color": "red", "index": 0, "text": "firing"},
                                              "ok": {"color": "green", "index": 1, "text": "ok"}}}]},
                             "overrides": named("exact", "approx")},
             "options": {"mergeValues": True, "showValue": "never", "rowHeight": 0.8, **LEGEND}}, 0, 32, 24, 6)

    b.row("Fidelity", 38)
    summary = RUN + "fidelity_summary.json"
    by_metric = f"$[{sel}]{{metric: value}}"
    b.panel({"type": "stat", "title": "Evaluations by outcome",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("true_positive", "true positive"), ("false_positive", "false positive"),
                 ("false_negative", "false negative"), ("true_negative", "true negative")))],
             "transformations": order("true positive", "false positive", "false negative", "true negative"),
             "fieldConfig": {"defaults": TEXT, "overrides": []},
             "options": STAT}, 0, 39, 6, 8)
    b.panel({"type": "stat", "title": "Episodes",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("episodes", "exact episodes"), ("missed_episodes", "missed episodes"),
                 ("mean_onset_delay", "mean onset delay (evaluations)"), ("breaks", "breaks"),
                 ("episode_coverage", "episode coverage")))],
             "transformations": order("exact episodes", "missed episodes", "mean onset delay (evaluations)",
                                      "breaks", "episode coverage"),
             "fieldConfig": {"defaults": TEXT, "overrides": [
                 {"matcher": {"id": "byName", "options": "episode coverage"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1}]}]},
             "options": STAT}, 6, 39, 6, 8)
    b.panel({"type": "table", "title": "Confusion matrix (evaluations)",
             "targets": [iq("A", summary,
                            f'( $s := $[{sel}]{{metric: value}}; ['
                            '{"exact": "firing", "approx firing": $s.true_positive, "approx ok": $s.false_negative}, '
                            '{"exact": "ok", "approx firing": $s.false_positive, "approx ok": $s.true_negative}] )',
                            [col("exact", "exact", "string"), col("approx firing", "approx firing", "number"),
                             col("approx ok", "approx ok", "number")])],
             "transformations": order("exact", "approx firing", "approx ok"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 12, 39, 6, 8)
    b.panel({"type": "table", "title": "Fidelity events",
             "targets": [iq("A", RUN + "fidelity_event.json",
                            or_empty(f"$[{sel}]"),
                            [T, col("metric", "event", "string"), col("value", "evaluations", "number")])],
             "transformations": order("time", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 18, 39, 6, 8)

    variables = [run_variable(use_cases),
                 variable("signal", summary, '$[metric="episodes"].labels', "signal")]
    annotations = [{"name": "fidelity events", "enable": True, "iconColor": "purple", "datasource": DS,
                    "target": iq("Anno", RUN + "fidelity_event.json",
                                 f'$[{sel}].{{"time": time, "text": metric & ": " & $string(value) & " evaluations"}}',
                                 [T, col("text", "text", "string")])}]
    return dashboard(uid, "Threshold alert", b, variables, annotations)


def sort_limit(field, limit):
    return [{"id": "sortBy", "options": {"sort": [{"field": field, "desc": True}]}},
            {"id": "limit", "options": {"limitField": limit}}]


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
    for i, (query, title, unit, low, high) in enumerate((("p95_cpu", "p95 CPU", "none", 0.3, 0.8),
                                                         ("p95_memory", "p95 memory", "bytes", 0.5e9, 1.5e9))):
        sel = f'{here} and labels.query="{query}"'
        b.panel({"type": "timeseries", "title": f"{title} of ${{container}}: exact vs approx",
                 "targets": [rows_of("A", "query_result", f'{sel} and labels.arm="exact"', "exact"),
                             rows_of("B", "query_result", f'{sel} and labels.arm="approx"', "approx")],
                 "fieldConfig": {"defaults": {**LINES, "unit": unit,
                                              "custom": {**LINES["custom"], "thresholdsStyle": {"mode": "dashed"}},
                                              "thresholds": {"mode": "absolute", "steps": [
                                                  {"color": "blue", "value": None}, {"color": "green", "value": low},
                                                  {"color": "red", "value": high}]}},
                                 "overrides": [by_ref("A", "blue", 2), by_ref("B", "orange")] + named("exact", "approx")},
                 "options": LEGEND}, 12 * i, 9, 12, 8)
    b.panel({"type": "timeseries", "title": "Relative error of approx for ${container}",
             "targets": [rows_of("A", "low_level", f'{here} and metric="relative_error"', "relative_error", by="query")],
             "fieldConfig": {"defaults": {**LINES, "unit": "percentunit", "displayName": "${__field.labels.query}"},
                             "overrides": []},
             "options": LEGEND}, 0, 17, 24, 6)

    b.row("Application: autoscaler", 23)
    b.panel({"type": "state-timeline", "title": "${container}: scaling decision",
             "targets": [rows_of("A", "app_state", f'{here} and labels.arm="exact"', "exact", "string"),
                         rows_of("B", "app_state", f'{here} and labels.arm="approx"', "approx", "string")],
             "fieldConfig": {"defaults": {"custom": {"fillOpacity": 80, "lineWidth": 0},
                                          "color": {"mode": "fixed", "fixedColor": "transparent"},
                                          "mappings": [{"type": "value", "options": {
                                              "up": {"color": "red", "index": 0, "text": "scale up"},
                                              "down": {"color": "blue", "index": 1, "text": "scale down"},
                                              "none": {"color": "green", "index": 2, "text": "no change"}}}]},
                             "overrides": named("exact", "approx")},
             "options": {"mergeValues": True, "showValue": "never", "rowHeight": 0.8, **LEGEND}}, 0, 24, 24, 6)

    b.row("Fidelity over all containers", 30)
    matrix = (f'( $s := $[{overall}]{{metric: value}}; ["up", "down", "none"].( $e := $; {{"exact": $e, '
              + ", ".join(f'"approx {a}": $lookup($s, "exact_" & $e & "_approx_{a}")' for a in ("up", "down", "none"))
              + "} ) )")
    b.panel({"type": "table", "title": "Decisions, exact by approx (container-evaluations)",
             "targets": [iq("A", summary, matrix, [col("exact", "exact", "string")]
                            + [col(f"approx {a}", f"approx {a}", "number") for a in ("up", "down", "none")])],
             "transformations": order("exact", "approx up", "approx down", "approx none"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 0, 31, 8, 7)
    fields = (("episodes", "exact episodes"), ("missed_episodes", "missed"), ("mean_onset_delay", "mean onset delay"),
              ("breaks", "breaks"), ("false_actions", "false actions"))
    per_action = (f'( $s := $[{overall}]{{metric: value}}; ["up", "down"].( $a := $; {{"action": $a, '
                  + ", ".join(f'"{text}": $lookup($s, $a & "_{metric}")' for metric, text in fields)
                  + "} ) )")
    b.panel({"type": "table", "title": "Scaling episodes by action",
             "targets": [iq("A", summary, per_action, [col("action", "action", "string")]
                            + [col(text, text, "number") for _, text in fields])],
             "transformations": order("action", *[text for _, text in fields]),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 8, 31, 9, 7)
    b.panel({"type": "table", "title": "Containers with the most disagreements",
             "targets": [iq("A", summary,
                            or_empty('$[metric="disagreements" and value > 0].'
                                     '{"container": labels.container, "disagreements": value}'),
                            [col("container", "container", "string"),
                             col("disagreements", "disagreements", "number")])],
             "transformations": sort_limit("disagreements", 20),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": [
                 {"matcher": {"id": "byName", "options": "container"},
                  "properties": [{"id": "links", "value": [{
                      "title": "Show this container",
                      "url": f"/d/{uid}/{uid}?var-run=${{run}}&var-container=${{__data.fields.container}}"
                             "&${__url_time_range}"}]}]}]},
             "options": {"showHeader": True}}, 17, 31, 7, 7)
    b.panel({"type": "table", "title": "Fidelity events of ${container}",
             "targets": [iq("A", RUN + "fidelity_event.json", or_empty(f"$[{here}]"),
                            [T, col("labels.action", "action", "string"), col("metric", "event", "string"),
                             col("value", "evaluations", "number")])],
             "transformations": order("time", "action", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 0, 38, 24, 6)

    container = variable("container", summary, '$[metric="disagreements"].labels', "container")
    container["sort"] = 1
    annotations = [{"name": "fidelity events", "enable": True, "iconColor": "purple", "datasource": DS,
                    "target": iq("Anno", RUN + "fidelity_event.json",
                                 or_empty(f'$[{here}].{{"time": time, "text": labels.action & " " & metric & ": " '
                                          '& $string(value) & " evaluations"}'),
                                 [T, col("text", "text", "string")])}]
    return dashboard(uid, "Autoscaler decision", b, [run_variable(use_cases), container], annotations)


def topk_dashboard():
    uid = "topk"
    use_cases = ["top_clients"]
    app = 'labels.app="topk_set"'
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

    b.row("Application: top clients", 18)
    b.panel({"type": "timeseries", "title": "Clients shared by the exact and approx top lists",
             "targets": [rows_of("A", "app_state", f'{app} and metric="overlap"', "in both"),
                         rows_of("B", "app_state", f'{app} and metric="approx_only"', "approx only")],
             "fieldConfig": {"defaults": {**LINES, "min": 0},
                             "overrides": [by_ref("A", "green", 2), by_ref("B", "orange")] + named("in both", "approx only")},
             "options": LEGEND}, 0, 19, 24, 6)
    b.panel({"type": "state-timeline", "title": "Membership in the top list, by client",
             "targets": [rows_of("A", "app_state", f'{app} and metric="membership"', "membership", by="client_id")],
             "fieldConfig": {"defaults": {"custom": {"fillOpacity": 80, "lineWidth": 0},
                                          "displayName": "${__field.labels.client_id}",
                                          "color": {"mode": "fixed", "fixedColor": "transparent"},
                                          "mappings": [{"type": "value", "options": {
                                              "3": {"color": "green", "index": 0, "text": "both"},
                                              "2": {"color": "red", "index": 1, "text": "exact only"},
                                              "1": {"color": "orange", "index": 2, "text": "approx only"},
                                              "0": {"color": "transparent", "index": 3, "text": "neither"}}}]},
                             "overrides": []},
             "options": {"mergeValues": True, "showValue": "never", "rowHeight": 0.8, **LEGEND}}, 0, 25, 24, 14)

    b.row("Fidelity", 39)
    by_metric = f"$[{app}]{{metric: value}}"
    b.panel({"type": "stat", "title": "Top list per evaluation",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("precision", "precision"), ("recall", "recall"), ("identical_sets", "identical lists"),
                 ("evaluations", "evaluations"), ("intruder_slots", "approx-only slots")))],
             "transformations": order("precision", "recall", "identical lists", "evaluations", "approx-only slots"),
             "fieldConfig": {"defaults": TEXT, "overrides": [
                 {"matcher": {"id": "byRegexp", "options": "precision|recall"},
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1}]}]},
             "options": STAT}, 0, 40, 8, 8)
    b.panel({"type": "stat", "title": "Clients entering the exact top list",
             "targets": [iq("A", summary, by_metric, by_metric_columns(
                 ("entries", "entries"), ("missed_entries", "missed"),
                 ("mean_onset_delay", "mean onset delay (evaluations)"), ("breaks", "breaks"),
                 ("false_member_spans", "approx-only spans")))],
             "transformations": order("entries", "missed", "mean onset delay (evaluations)", "breaks",
                                      "approx-only spans"),
             "fieldConfig": {"defaults": TEXT, "overrides": []},
             "options": STAT}, 8, 40, 8, 8)
    b.panel({"type": "table", "title": "Fidelity events",
             "targets": [iq("A", RUN + "fidelity_event.json", or_empty(f"$[{app}]"),
                            [T, col("labels.client_id", "client", "string"), col("metric", "event", "string"),
                             col("value", "evaluations", "number")])],
             "transformations": order("time", "client", "event", "evaluations"),
             "fieldConfig": {"defaults": {"custom": {"minWidth": 60}}, "overrides": []},
             "options": {"showHeader": True}}, 16, 40, 8, 8)
    return dashboard(uid, "Top clients", b, [run_variable(use_cases)])


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
             "targets": [iq("A", INDEX, or_empty(rows), [col(k, k, "string") for k in text]
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
                  "properties": [{"id": "unit", "value": "percentunit"}, {"id": "decimals", "value": 1},
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
    for board in (overview_dashboard(), alert_dashboard(), decision_dashboard(), topk_dashboard()):
        with open(os.path.join(OUT, board["uid"] + ".json"), "w") as f:
            json.dump(board, f, indent=2)
            f.write("\n")


if __name__ == "__main__":
    main()
