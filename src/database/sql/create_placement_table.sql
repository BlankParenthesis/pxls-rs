CREATE TABLE IF NOT EXISTS `placement`(
	`id`        INTEGER PRIMARY KEY AUTOINCREMENT,
	`board`     INTEGER NOT NULL,
	`position`  INTEGER NOT NULL,
	`color`     INTEGER NOT NULL,
	`timestamp` INTEGER NOT NULL,
	`user_id`   TEXT NULL,
	FOREIGN KEY (`board`) REFERENCES `board`(`id`),
	FOREIGN KEY (`board`, `color`) REFERENCES `color`(`board`, `index`)
);