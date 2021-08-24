CREATE TABLE IF NOT EXISTS `board` (
	`id`         INTEGER PRIMARY KEY AUTOINCREMENT,
	`name`       TEXT NOT NULL,
	`created_at` INTEGER NOT NULL,
	`shape`      TEXT NOT NULL,
  	`palette`    INTEGER NOT NULL,
	`mask`       BLOB NOT NULL,
	`initial`    BLOB NOT NULL,
	FOREIGN KEY (`palette`) REFERENCES `palette`(`id`)
	UNIQUE(`id`, `palette`)
);