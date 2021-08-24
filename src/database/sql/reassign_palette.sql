BEGIN TRANSACTION;

UPDATE `board`
SET `palette` = ?2
WHERE `id` = ?1;

UPDATE `placement`
SET `palette` = ?2
WHERE `board` = ?1;

COMMIT;