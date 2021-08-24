CREATE TABLE IF NOT EXISTS `placement`(
	`board`     INTEGER NOT NULL,
	`position`  INTEGER NOT NULL,
	`color`     INTEGER NOT NULL,
	`timestamp` INTEGER NOT NULL,
  	`palette`   INTEGER NOT NULL,
	FOREIGN KEY (`board`, `palette`) REFERENCES `board`(`id`, `palette`) DEFERRABLE INITIALLY DEFERRED,
	FOREIGN KEY (`palette`, `color`) REFERENCES `color`(`palette`, `index`)
);