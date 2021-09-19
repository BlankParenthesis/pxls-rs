SELECT `id`, `position`, `color`, `timestamp`
FROM `placement`
WHERE `board` = ?1 AND (`timestamp`, `id`) >= (?2, ?3)
ORDER BY `timestamp`, `id`
LIMIT ?4;