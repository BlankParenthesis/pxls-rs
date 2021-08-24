CREATE TABLE IF NOT EXISTS `color` (
	`palette` INTEGER,
	`index`   INTEGER,
	`name`    TEXT,
	`value`   INTEGER,
	UNIQUE(`palette`, `index`),
	FOREIGN KEY (`palette`) REFERENCES `palette`(`id`)
);