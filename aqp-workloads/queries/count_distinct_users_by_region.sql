SELECT region, COUNT(DISTINCT user_id) AS users
FROM events
GROUP BY region;
