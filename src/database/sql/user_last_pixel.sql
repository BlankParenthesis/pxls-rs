SELECT `id`, `position`, `color`, `timestamp`
FROM `placement`
WHERE `board` = ?1 AND `user_id` = ?2
ORDER BY `timestamp` DESC, `id` DESC
LIMIT 1;