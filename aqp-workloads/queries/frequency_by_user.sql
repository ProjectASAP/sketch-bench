SELECT user_id, COUNT(*) AS frequency
FROM events
GROUP BY user_id;
