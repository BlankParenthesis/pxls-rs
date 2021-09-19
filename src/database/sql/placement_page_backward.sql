SELECT `id`, `position`, `color`, `timestamp`
FROM `placement`
WHERE `board` = ?1
AND (`timestamp`, `id`) < (?2, ?3)
ORDER BY `timestamp` DESC, `id` DESC
LIMIT ?4;