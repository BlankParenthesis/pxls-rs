CREATE TABLE IF NOT EXISTS `board` (
	`id`         INTEGER PRIMARY KEY AUTOINCREMENT,
	`name`       TEXT NOT NULL,
	`created_at` INTEGER NOT NULL,
	`shape`      TEXT NOT NULL,
	`mask`       BLOB NOT NULL,
	`initial`    BLOB NOT NULL
);