-- revert shape format

ALTER TABLE "board"
ADD COLUMN "shape_old" TEXT;

-- NOTE: takes the last shape element since that's what should
-- match the sector data sizes. 
-- If the number of dimensions is not two, the server is going
-- to have a bad time regardless.
UPDATE "board"
SET "shape_old" = '[' || CAST(TO_JSON(shape)->-1 as TEXT) || ']';

ALTER TABLE "board"
ALTER COLUMN "shape_old"
SET NOT NULL;

ALTER TABLE "board"
DROP COLUMN "shape";

ALTER TABLE "board"
RENAME COLUMN "shape_old" TO "shape";

-- revert sector table, move back data

ALTER TABLE "board"
ADD COLUMN "mask" BYTEA,
ADD COLUMN "initial" BYTEA;

-- NOTE: this will only convert back the first sector.
-- The old format didn't store multiple sectors,
-- so this is the best we can do really.
UPDATE "board"
SET "mask" = "board_sector"."mask",
    "initial" = "board_sector"."initial"
FROM "board_sector"
WHERE "board"."id" = "board_sector"."board"
AND "board_sector"."sector" = 0;

ALTER TABLE "board"
ALTER COLUMN "mask" SET NOT NULL,
ALTER COLUMN "initial" SET NOT NULL;

DROP TABLE "board_sector";