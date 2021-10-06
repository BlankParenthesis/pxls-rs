-- add sector table and move data over

CREATE TABLE "board_sector" (
	"board"   INTEGER NOT NULL,
	"sector"  INTEGER NOT NULL,
	"mask"    BYTEA NOT NULL,
	"initial" BYTEA NOT NULL,
	PRIMARY KEY ("board", "sector"),
	FOREIGN KEY ("board") REFERENCES "board"("id")
);

INSERT INTO "board_sector"
SELECT "id", 0, "mask", "initial" FROM "board";

ALTER TABLE "board"
DROP COLUMN "mask",
DROP COLUMN "initial";

-- replace shape format

ALTER TABLE "board"
ADD COLUMN "shape_new" INTEGER[][];

UPDATE "board"
SET "shape_new" = CAST(REPLACE(REPLACE("shape", '[', '{'), ']', '}') AS INTEGER[][]);

ALTER TABLE "board"
ALTER COLUMN "shape_new" SET NOT NULL;

ALTER TABLE "board"
DROP COLUMN "shape";

ALTER TABLE "board"
RENAME COLUMN "shape_new" TO "shape";