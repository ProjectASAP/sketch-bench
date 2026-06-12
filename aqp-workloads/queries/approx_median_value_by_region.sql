SELECT region, approx_median(value) AS p50_value
FROM events
GROUP BY region;
