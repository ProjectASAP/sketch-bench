# Error vs N up to 1e9

Drawn by `scripts/plot_saturation_curves.py` from `out_1e9/saturation_curve.csv`
(the 1e9 subset in `docs/saturation_study.md`): one config per sketch from the
grid (rows=3 cols=1024 for CMS/CountSketch/top-k, lg_k=12, k=200, alpha=0.01),
N from 1e3 to 1e9, 3 seeds, Pareto values floored to i64 (scale 1000). Panels,
bands, bounds and N_sat markers read as in `../reduced_run/README.md`; the
1e9 findings are summarised in the doc's "Results (1e9 subset)" section.
