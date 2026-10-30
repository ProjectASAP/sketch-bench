#!/usr/bin/env python3

from approx_rows import row
from threshold_alert import alert_fidelity, relative_errors, series

APP = "comparison_alert"


def derive(query_rows, signal, left, right, factor):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    (run_id,) = run_ids
    left_labels, _, times, left_exact, left_approx = series(query_rows, left)
    right_labels, _, right_times, right_exact, right_approx = series(query_rows, right)
    if times != right_times:
        raise ValueError(f"queries {left} and {right} are not evaluated at the same times")
    out = relative_errors(run_id, left_labels, times, left_exact, left_approx)
    out += relative_errors(run_id, right_labels, times, right_exact, right_approx)
    app = {"use_case": left_labels["use_case"], "signal": signal, "app": APP}
    exact = [le / re for le, re in zip(left_exact, right_exact)]
    approx = [la / ra for la, ra in zip(left_approx, right_approx)]
    for t, e, a in zip(times, exact, approx):
        out.append(row(t, "app_state", run_id, {**app, "arm": "exact"}, "ratio", e, "ratio"))
        out.append(row(t, "app_state", run_id, {**app, "arm": "approx"}, "ratio", a, "ratio"))
        out.append(row(t, "app_state", run_id, app, "tolerance", factor, "ratio"))
    return out + alert_fidelity(run_id, app, times, [e > factor for e in exact], [a > factor for a in approx])
