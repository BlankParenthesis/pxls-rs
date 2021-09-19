SELECT `id`, `position`, `color`, `timestamp`
FROM `placement`
WHERE `board` = ?1 AND `position` = ?2
ORDER BY `timestamp` DESC, `id` DESC
LIMIT 1;