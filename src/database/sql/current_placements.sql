SELECT DISTINCT `id`, `position`, `color`, max(`timestamp`)
FROM `placement`
WHERE `board` = ?1
GROUP BY `position`
ORDER BY max(`timestamp`) DESC