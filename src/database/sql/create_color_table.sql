CREATE TABLE IF NOT EXISTS `color` (
	`board`   INTEGER NOT NULL,
	`index`   INTEGER NOT NULL,
	`name`    TEXT NOT NULL,
	`value`   INTEGER NOT NULL,
	UNIQUE(`board`, `index`),
	FOREIGN KEY (`board`) REFERENCES `board`(`id`)
);