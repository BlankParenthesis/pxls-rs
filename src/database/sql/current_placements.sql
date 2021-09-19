SELECT `id`, `position`, `color`, `timestamp`
FROM (SELECT * FROM `placement` ORDER BY `timestamp` DESC, `id` DESC)
WHERE `board` = ?1
GROUP BY `position`;