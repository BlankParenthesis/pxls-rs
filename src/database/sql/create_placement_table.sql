CREATE TABLE IF NOT EXISTS `placement`(
	`board`     INTEGER,
	`position`  INTEGER,
	`color`     INTEGER,
	`timestamp` INTEGER,
  	`palette`   INTEGER,
	FOREIGN KEY (`board`, `palette`) REFERENCES `board`(`id`, `palette`) DEFERRABLE INITIALLY DEFERRED,
	FOREIGN KEY (`palette`, `color`) REFERENCES `color`(`palette`, `index`)
);