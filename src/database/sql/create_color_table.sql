CREATE TABLE IF NOT EXISTS `color` (
	`palette` INTEGER NOT NULL,
	`index`   INTEGER NOT NULL,
	`name`    TEXT NOT NULL,
	`value`   INTEGER NOT NULL,
	UNIQUE(`palette`, `index`),
	FOREIGN KEY (`palette`) REFERENCES `palette`(`id`)
);