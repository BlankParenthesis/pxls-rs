CREATE TABLE IF NOT EXISTS `board` (
	`id`         INTEGER PRIMARY KEY AUTOINCREMENT,
	`name`       TEXT,
	`created_at` INTEGER,
	`shape`      TEXT,
  	`palette`    INTEGER REFERENCES `palette`(`id`),
	UNIQUE(`id`, `palette`)
);